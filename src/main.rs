mod config;
mod error;
mod gdb;
mod mi;
mod models;
mod tools;
mod ui;

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use crossterm::event::EventStream;
use error::{AppError, AppResult};
use futures::StreamExt;
use gdb::GDBManager;
use mcp_core::server::{Server, ServerProtocolBuilder};
use mcp_core::transport::{ServerSseTransport, ServerStdioTransport, Transport};
use mcp_core::types::ServerCapabilities;
use models::{ASM, BT, MemoryMapping, MemoryType, ResolveSymbol, TrackedRegister};
use ratatui::Terminal;
use ratatui::crossterm::event::{DisableMouseCapture, Event, KeyCode};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::prelude::Backend;
use ratatui::widgets::ScrollbarState;
use serde_json::json;
use tokio::sync::{Mutex, mpsc, oneshot};
use tools::GDB_MANAGER;
use tracing::{debug, error, info, warn};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use ui::hexdump::HEXDUMP_WIDTH;

#[derive(Copy, Clone, PartialEq, Eq, ValueEnum, Debug)]
enum TransportType {
    Stdio,
    Sse,
}

impl FromStr for TransportType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "stdio" => Ok(TransportType::Stdio),
            "sse" => Ok(TransportType::Sse),
            _ => Err(format!("Invalid transport type: {}", s)),
        }
    }
}

pub static TRANSPORT: LazyLock<Mutex<Option<Arc<Box<dyn Transport>>>>> =
    LazyLock::new(|| Mutex::new(None));

fn resolve_home(path: &str) -> Option<PathBuf> {
    if path.starts_with("~/") {
        if let Ok(home) = env::var("HOME") {
            return Some(Path::new(&home).join(&path[2..]));
        }
        None
    } else {
        Some(PathBuf::from(path))
    }
}

#[derive(Parser, Debug)]
#[command(
    author,
    version,
    about = "A GDB/MI protocol server based on the MCP protocol, providing remote application debugging capabilities",
    long_about = "MCP GDB Server - A GDB/MI protocol server that provides remote debugging capabilities via the MCP protocol.\n\nUsage Examples:\n\n  # Start server in stdio mode (default)\n  mcp-server-gdb\n\n  # Start server in SSE mode\n  mcp-server-gdb --transport sse\n\n  # Start server with TUI enabled (requires SSE transport)\n  mcp-server-gdb --transport sse --enable-tui\n\n  # Start server with debug log level\n  mcp-server-gdb --log-level debug\n\nRemote Debugging:\n  1. Start gdbserver on target machine: gdbserver :1234 /path/to/program\n  2. Connect via MCP tool: create_session with remote_host and remote_port parameters\n\nAvailable MCP Tools:\n  - create_session: Create a new GDB debugging session\n  - get_session: Get session information\n  - get_all_sessions: Get all sessions\n  - close_session: Close a session\n  - start_debugging: Start program execution\n  - stop_debugging: Stop program execution\n  - set_breakpoint: Set a breakpoint\n  - get_breakpoints: Get all breakpoints\n  - delete_breakpoint: Delete breakpoints\n  - get_stack_frames: Get stack frames\n  - get_local_variables: Get local variables\n  - get_registers: Get register values\n  - read_memory: Read memory contents\n  - continue_execution: Continue execution\n  - step_execution: Step into function\n  - next_execution: Step over function\n  - connect_remote: Connect to gdbserver\n  - disconnect_remote: Disconnect from gdbserver\n  - load_symbols: Load symbols from file\n"
)]
struct Args {
    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", help = "Logging level: trace, debug, info, warn, or error")]
    log_level: String,

    /// Transport type to use
    #[arg(
        value_enum,
        default_value_t = TransportType::Stdio,
        required_if_eq("enable_tui", "true"),
        value_parser = clap::builder::ValueParser::new(|s: &str| -> Result<TransportType, String> {
            let t = s.parse::<TransportType>()?;
            if t == TransportType::Stdio && std::env::args().any(|arg| arg == "--enable-tui") {
                Err("When TUI is enabled, transport must be SSE".to_string())
            } else {
                Ok(t)
            }
        }),
        help = "Transport type: stdio (default) or sse. SSE is required when TUI is enabled"
    )]
    transport: TransportType,

    /// Enable terminal user interface (requires SSE transport)
    #[arg(long, help = "Enable interactive terminal UI. Requires --transport sse")]
    enable_tui: bool,
}

#[derive(Copy, Clone, Default, PartialEq)]
enum Mode {
    #[default]
    All,
    OnlyRegister,
    OnlyStack,
    OnlyInstructions,
    OnlyOutput,
    OnlyMapping,
    OnlyHexdump,
}

impl Mode {
    pub fn next(&self) -> Self {
        match self {
            Mode::All => Mode::OnlyRegister,
            Mode::OnlyRegister => Mode::OnlyStack,
            Mode::OnlyStack => Mode::OnlyInstructions,
            Mode::OnlyInstructions => Mode::OnlyOutput,
            Mode::OnlyOutput => Mode::OnlyMapping,
            Mode::OnlyMapping => Mode::OnlyHexdump,
            Mode::OnlyHexdump => Mode::All,
        }
    }
}

/// An endian
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Endian {
    /// Little endian
    Little,
    /// Big endian
    Big,
}

#[derive(Default)]
pub struct MyScrollState {
    pub scroll: usize,
    pub state: ScrollbarState,
}

#[derive(Default)]
struct App {
    gdb: GDBManager,
    /// -32 bit mode
    bit32: bool,
    /// Current filepath of .text
    filepath: Option<PathBuf>,
    /// Current endian
    endian: Option<Endian>,
    /// Current display mode
    mode: Mode,
    /// Memory map TUI
    memory_map: Option<Vec<MemoryMapping>>,
    memory_map_scroll: MyScrollState,
    /// Current $pc
    current_pc: u64, // TODO: replace with AtomicU64?
    /// All output from gdb
    output: Vec<String>,
    output_scroll: MyScrollState,
    /// Saved output such as (gdb) or > from gdb
    stream_output_prompt: String,
    /// Register TUI
    register_changed: Vec<u8>,
    registers: Vec<TrackedRegister>,
    /// Saved Stack
    stack: BTreeMap<u64, ResolveSymbol>,
    /// Saved ASM
    asm: Vec<ASM>,
    /// Hexdump
    hexdump: Option<(u64, Vec<u8>)>,
    hexdump_scroll: MyScrollState,
    /// Right side of status in TUI
    async_result: String,
    /// Left side of status in TUI
    status: String,
    bt: Vec<BT>,
    /// Exit the app
    _exit: bool,
}

impl App {
    // Parse a "file filepath" command and save
    fn save_filepath(&mut self, val: &str) {
        let filepath: Vec<&str> = val.split_whitespace().collect();
        let filepath = resolve_home(filepath[1]).expect("Failed to resolve home directory");
        // debug!("filepath: {filepath:?}");
        self.filepath = Some(filepath);
    }

    pub async fn find_first_heap(&self) -> Option<MemoryMapping> {
        self.memory_map.as_ref()?.iter().find(|a| a.is_heap()).cloned()
    }

    pub async fn find_first_stack(&self) -> Option<MemoryMapping> {
        self.memory_map.as_ref()?.iter().find(|a| a.is_stack()).cloned()
    }

    pub fn classify_val(&self, val: u64, filepath: &Path) -> MemoryType {
        if val != 0 {
            // look through, add see if the value is part of the stack
            // trace!("{:02x?}", memory_map);
            if let Some(memory_map) = self.memory_map.as_ref() {
                for r in memory_map {
                    if r.contains(val) {
                        if r.is_stack() {
                            return MemoryType::Stack;
                        }
                        if r.is_heap() {
                            return MemoryType::Heap;
                        }
                        if r.is_path(filepath) || r.is_exec() {
                            // TODO(23): This could be expanded to all segments loaded in
                            // as executable
                            return MemoryType::Exec;
                        }
                    }
                }
            }
        }
        MemoryType::Unknown
    }
}

async fn init_logging(log_level: &str) {
    let file_appender = RollingFileAppender::new(Rotation::DAILY, "logs", "mcp-gdb.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::try_new(log_level).unwrap_or_else(|_| EnvFilter::new("info"))
        }))
        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
        .init();
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    dotenv::dotenv().ok();

    let args = Args::parse();

    let file_appender = RollingFileAppender::new(Rotation::DAILY, "logs", "mcp-gdb.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::try_new(&args.log_level).unwrap_or_else(|_| EnvFilter::new("info"))
        }))
        .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
        .init();

    let config = config::Config::default();
    debug!("config: {:?}", config);

    info!("Starting MCP GDB Server on port {}", config.server_port);

    let app = Arc::new(Mutex::new(Default::default()));

    let ui_handle = if args.enable_tui {
        enable_raw_mode()?;
        execute!(std::io::stdout(), EnterAlternateScreen)?;
        match ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(std::io::stdout())) {
            Ok(terminal) => {
                let terminal = Arc::new(Mutex::new(terminal));
                let (quit_sender, quit_receiver) = oneshot::channel();
                let app_clone = app.clone();
                let terminal_for_tui = terminal.clone();
                let tui_handle = tokio::spawn(async move {
                    if let Err(e) = run_app(terminal_for_tui, app_clone).await {
                        error!("failed to run app: {}", e);
                    } else {
                        let _ = quit_sender.send(());
                    }
                });
                Some((terminal, tui_handle, quit_receiver))
            }
            Err(e) => {
                warn!("Failed to initialize terminal: {}", e);
                None
            }
        }
    } else {
        debug!("TUI disabled by command line argument");
        None
    };

    tools::init_gdb_manager();

    let server_protocol =
        Server::builder("MCP Server GDB".to_string(), env!("CARGO_PKG_VERSION").to_string())
            .capabilities(ServerCapabilities {
                tools: Some(json!({
                    "listChanged": false,
                })),
                ..Default::default()
            });

    let server_protocol = register_tools(server_protocol).build();

    let transport = match args.transport {
        TransportType::Stdio => {
            let transport = Arc::new(
                Box::new(ServerStdioTransport::new(server_protocol)) as Box<dyn Transport>
            );
            {
                let mut transport_guard = TRANSPORT.lock().await;
                *transport_guard = Some(transport.clone());
            }
            transport
        }
        TransportType::Sse => {
            let transport = Arc::new(Box::new(ServerSseTransport::new(
                config.server_ip,
                config.server_port,
                server_protocol,
            )) as Box<dyn Transport>);
            {
                let mut transport_guard = TRANSPORT.lock().await;
                *transport_guard = Some(transport.clone());
            }
            transport
        }
    };

    let transport_clone = transport.clone();
    let transport_handle = tokio::spawn(async move {
        if let Err(e) = transport_clone.open().await {
            error!("transport error: {}", e);
        }
    });

    if let Some((terminal, tui_handle, quit_receiver)) = ui_handle {
        if let Err(e) = quit_receiver.await {
            error!("failed to receive quit signal: {}", e);
        }

        tui_handle.abort();

        disable_raw_mode()?;
        let mut terminal = terminal.lock().await;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
        terminal.show_cursor()?;
        debug!("TUI closed");
    } else {
        debug!("waiting for transport to complete");
        if let Err(e) = transport_handle.await {
            error!("transport task error: {}", e);
        }
        match tools::GDB_MANAGER.get_all_sessions().await {
            Ok(sessions) => {
                for session in sessions {
                    let _ = tools::GDB_MANAGER.close_session(&session.id).await;
                }
            }
            Err(e) => {
                error!("failed to get sessions for cleanup: {}", e);
            }
        }
        info!("MCP GDB Server shutdown complete");
        return Ok(());
    }

    if let Err(e) = transport.close().await {
        error!("failed to close transport: {}", e);
    }
    transport_handle.abort();

    match tools::GDB_MANAGER.get_all_sessions().await {
        Ok(sessions) => {
            for session in sessions {
                let _ = tools::GDB_MANAGER.close_session(&session.id).await;
            }
        }
        Err(e) => {
            error!("failed to get sessions for cleanup: {}", e);
        }
    }

    info!("MCP GDB Server shutdown complete");
    Ok(())
}

fn scroll_down(n: usize, scroll: &mut MyScrollState, len: usize) {
    if scroll.scroll < len.saturating_sub(1) {
        scroll.scroll += n;
        scroll.state = scroll.state.position(scroll.scroll);
    }
}

fn scroll_up(n: usize, scroll: &mut MyScrollState) {
    if scroll.scroll > n {
        scroll.scroll -= n;
    } else {
        scroll.scroll = 0;
    }
    scroll.state = scroll.state.position(scroll.scroll);
}

async fn run_app<B: Backend + Send + 'static>(
    terminal: Arc<Mutex<Terminal<B>>>,
    app: Arc<Mutex<App>>,
) -> AppResult<()> {
    let app_clone1 = app.clone();
    let app_clone2 = app.clone();
    let mut reader = EventStream::new();
    let (tx, mut rx) = mpsc::channel(100);

    let event_loop = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let Event::Key(key) = event {
                debug!("key >>> {:?}", key);
                let mut app = app_clone1.lock().await;
                match key.code {
                    KeyCode::Tab => {
                        app.mode = app.mode.next();
                    }
                    KeyCode::F(1) => {
                        app.mode = Mode::All;
                    }
                    KeyCode::F(2) => {
                        app.mode = Mode::OnlyRegister;
                    }
                    KeyCode::F(3) => {
                        app.mode = Mode::OnlyStack;
                    }
                    KeyCode::F(4) => {
                        app.mode = Mode::OnlyInstructions;
                    }
                    KeyCode::F(5) => {
                        app.mode = Mode::OnlyOutput;
                    }
                    KeyCode::F(6) => {
                        app.mode = Mode::OnlyMapping;
                    }
                    KeyCode::F(7) => {
                        app.mode = Mode::OnlyHexdump;
                    }
                    // output
                    KeyCode::Char('g') if app.mode == Mode::OnlyOutput => {
                        app.output_scroll.scroll = 0;
                        app.output_scroll.state = app.output_scroll.state.position(0);
                    }
                    KeyCode::Char('G') if app.mode == Mode::OnlyOutput => {
                        let len = app.output.len();
                        app.output_scroll.scroll = len;
                        app.output_scroll.state.last();
                    }
                    KeyCode::Char('j') if app.mode == Mode::OnlyOutput => {
                        let len = app.output.len();
                        scroll_down(1, &mut app.output_scroll, len);
                    }
                    KeyCode::Char('k') if app.mode == Mode::OnlyOutput => {
                        scroll_up(1, &mut app.output_scroll);
                    }
                    KeyCode::Char('J') if app.mode == Mode::OnlyOutput => {
                        let len = app.output.len();
                        scroll_down(50, &mut app.output_scroll, len);
                    }
                    KeyCode::Char('K') if app.mode == Mode::OnlyOutput => {
                        scroll_up(50, &mut app.output_scroll);
                    }
                    // memory mapping
                    KeyCode::Char('g') if app.mode == Mode::OnlyMapping => {
                        app.memory_map_scroll.scroll = 0;
                        app.memory_map_scroll.state = app.memory_map_scroll.state.position(0);
                    }
                    KeyCode::Char('G') if app.mode == Mode::OnlyMapping => {
                        if let Some(memory) = app.memory_map.as_ref() {
                            let len = memory.len();
                            let memory_map_scroll = &mut app.memory_map_scroll;
                            memory_map_scroll.scroll = len;
                            memory_map_scroll.state.last();
                        }
                    }
                    KeyCode::Char('j') if app.mode == Mode::OnlyMapping => {
                        if let Some(memory) = app.memory_map.as_ref() {
                            let len = memory.len() / HEXDUMP_WIDTH;
                            scroll_down(1, &mut app.memory_map_scroll, len);
                        }
                    }
                    KeyCode::Char('k') if app.mode == Mode::OnlyMapping => {
                        scroll_up(1, &mut app.memory_map_scroll);
                    }
                    KeyCode::Char('J') if app.mode == Mode::OnlyMapping => {
                        if let Some(memory) = app.memory_map.as_ref() {
                            let len = memory.len() / HEXDUMP_WIDTH;
                            scroll_down(50, &mut app.memory_map_scroll, len);
                        }
                    }
                    KeyCode::Char('K') if app.mode == Mode::OnlyMapping => {
                        scroll_up(50, &mut app.memory_map_scroll);
                    }
                    // hexdump
                    KeyCode::Char('g') if app.mode == Mode::OnlyHexdump => {
                        app.hexdump_scroll.scroll = 0;
                        app.hexdump_scroll.state = app.hexdump_scroll.state.position(0);
                    }
                    KeyCode::Char('G') if app.mode == Mode::OnlyHexdump => {
                        if let Some(hexdump) = app.hexdump.as_ref() {
                            let len = hexdump.1.len() / HEXDUMP_WIDTH;
                            let hexdump_scroll = &mut app.hexdump_scroll;
                            hexdump_scroll.scroll = len;
                            hexdump_scroll.state.last();
                        }
                    }
                    KeyCode::Char('H') if app.mode == Mode::OnlyHexdump => {
                        if let Some(find_heap) = app.find_first_heap().await {
                            let _memory = GDB_MANAGER
                                .read_memory(
                                    "",
                                    Some(find_heap.start_address as isize),
                                    "0".to_string(),
                                    find_heap.size as usize,
                                )
                                .await?;

                            app.hexdump_scroll.scroll = 0;
                            app.hexdump_scroll.state = app.hexdump_scroll.state.position(0);
                        }
                    }
                    KeyCode::Char('T') if app.mode == Mode::OnlyHexdump => {
                        if let Some(find_stack) = app.find_first_stack().await {
                            let _memory = GDB_MANAGER
                                .read_memory(
                                    "",
                                    Some(find_stack.start_address as isize),
                                    "0".to_string(),
                                    find_stack.size as usize,
                                )
                                .await?;
                            // TODO: print memory

                            // reset position
                            app.hexdump_scroll.scroll = 0;
                            app.hexdump_scroll.state = app.hexdump_scroll.state.position(0);
                        }
                    }
                    KeyCode::Char('j') if app.mode == Mode::OnlyHexdump => {
                        if let Some(hexdump) = app.hexdump.as_ref() {
                            let len = hexdump.1.len() / HEXDUMP_WIDTH;
                            scroll_down(1, &mut app.hexdump_scroll, len);
                        }
                    }
                    KeyCode::Char('k') if app.mode == Mode::OnlyHexdump => {
                        scroll_up(1, &mut app.hexdump_scroll);
                    }
                    KeyCode::Char('J') if app.mode == Mode::OnlyHexdump => {
                        if let Some(hexdump) = app.hexdump.as_ref() {
                            let len = hexdump.1.len() / HEXDUMP_WIDTH;
                            scroll_down(50, &mut app.hexdump_scroll, len);
                        }
                    }
                    KeyCode::Char('K') if app.mode == Mode::OnlyHexdump => {
                        scroll_up(1, &mut app.hexdump_scroll);
                    }
                    _ => {
                        // app.input.handle_event(&Event::Key(key));
                    }
                }
            }
        }
        let mut app = app.lock().await;
        app._exit = true;
        Ok::<(), AppError>(())
    });

    let draw_loop = tokio::task::spawn_blocking(move || {
        loop {
            {
                let mut terminal = terminal.blocking_lock();
                let mut app = app_clone2.blocking_lock();
                if app._exit {
                    break;
                }
                if let Err(e) = terminal.draw(|f| {
                    ui::ui(f, &mut app);
                }) {
                    error!("failed to draw: {}", e);
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    });

    // Event collection task
    while let Some(Ok(event)) = reader.next().await {
        debug!("event <<< {:?}", event);
        if let Event::Key(key) = event {
            if key.code == KeyCode::Char('q') {
                drop(tx);
                break;
            }
            if let Err(e) = tx.send(event).await {
                error!("failed to send event: {}", e);
                break;
            }
        }
    }

    // Wait for processor to finish
    if let Err(e) = event_loop.await {
        error!("event processor error: {}", e);
    }

    // Wait for draw task to finish
    if let Err(e) = draw_loop.await {
        error!("failed to wait for draw task to finish: {}", e);
    }

    Ok(())
}

/// Register all debugging tools to the server
fn register_tools(builder: ServerProtocolBuilder) -> ServerProtocolBuilder {
    builder
        .register_tool(tools::CreateSessionTool::tool(), tools::CreateSessionTool::call())
        .register_tool(tools::GetSessionTool::tool(), tools::GetSessionTool::call())
        .register_tool(tools::GetAllSessionsTool::tool(), tools::GetAllSessionsTool::call())
        .register_tool(tools::CloseSessionTool::tool(), tools::CloseSessionTool::call())
        .register_tool(tools::StartDebuggingTool::tool(), tools::StartDebuggingTool::call())
        .register_tool(tools::StopDebuggingTool::tool(), tools::StopDebuggingTool::call())
        .register_tool(tools::GetBreakpointsTool::tool(), tools::GetBreakpointsTool::call())
        .register_tool(tools::SetBreakpointTool::tool(), tools::SetBreakpointTool::call())
        .register_tool(tools::DeleteBreakpointTool::tool(), tools::DeleteBreakpointTool::call())
        .register_tool(tools::GetStackFramesTool::tool(), tools::GetStackFramesTool::call())
        .register_tool(tools::GetLocalVariablesTool::tool(), tools::GetLocalVariablesTool::call())
        .register_tool(tools::ContinueExecutionTool::tool(), tools::ContinueExecutionTool::call())
        .register_tool(tools::StepExecutionTool::tool(), tools::StepExecutionTool::call())
        .register_tool(tools::NextExecutionTool::tool(), tools::NextExecutionTool::call())
        .register_tool(tools::GetRegistersTool::tool(), tools::GetRegistersTool::call())
        .register_tool(tools::GetRegisterNamesTool::tool(), tools::GetRegisterNamesTool::call())
        .register_tool(tools::ReadMemoryTool::tool(), tools::ReadMemoryTool::call())
        .register_tool(tools::ConnectRemoteTool::tool(), tools::ConnectRemoteTool::call())
        .register_tool(tools::DisconnectRemoteTool::tool(), tools::DisconnectRemoteTool::call())
        .register_tool(tools::LoadSymbolsTool::tool(), tools::LoadSymbolsTool::call())
}
