# MCP GDB Server 使用说明

## 概述

MCP GDB Server 是一个基于 Rust 开发的 GDB 调试服务器，通过 MCP (Model Context Protocol) 协议提供远程 GDB 调试能力。该服务器支持：

- **本地调试**：直接调试本地可执行文件
- **远程调试**：连接到 gdbserver 进行远程目标调试
- **TUI 界面**：可选的终端用户界面
- **多会话管理**：支持同时管理多个调试会话

## 安装与运行

### 前置依赖

- Rust 1.70+
- GDB (GNU Debugger)

### 构建

```bash
cargo build --release
```

### 运行

```bash
# 标准输入输出模式（默认）
./target/release/mcp_server_gdb

# SSE 模式（支持 TUI）
./target/release/mcp_server_gdb --transport sse

# 启用 TUI 界面
./target/release/mcp_server_gdb --transport sse --enable-tui
```

### 命令行参数

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `--log-level` | String | info | 日志级别 (trace, debug, info, warn, error) |
| `--transport` | Stdio/Sse | Stdio | 传输类型 |
| `--enable-tui` | bool | false | 启用终端用户界面 |

## MCP 工具列表

### 会话管理

#### 1. create_session

创建一个新的 GDB 调试会话。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `program` | String | 否 | 要调试的可执行文件路径 |
| `nh` | bool | 否 | 不读取 ~/.gdbinit |
| `nx` | bool | 否 | 不读取任何 .gdbinit |
| `quiet` | bool | 否 | 启动时不打印版本信息 |
| `cd` | String | 否 | 切换工作目录 |
| `bps` | u32 | 否 | 远程调试波特率 |
| `symbol_file` | String | 否 | 符号文件路径 |
| `core_file` | String | 否 | core dump 文件路径 |
| `proc_id` | u32 | 否 | 附加到运行中的进程 PID |
| `command` | String | 否 | 执行 GDB 命令文件 |
| `source_dir` | String | 否 | 源文件搜索目录 |
| `args` | Vec<String> | 否 | 传递给被调试程序的参数 |
| `tty` | String | 否 | 被调试程序的 TTY |
| `gdb_path` | String | 否 | GDB 可执行文件路径 |
| `remote_target_type` | String | 否 | 远程目标类型：`remote` 或 `extended-remote` |
| `remote_host` | String | 否 | gdbserver 主机地址 |
| `remote_port` | u16 | 否 | gdbserver 端口号 |

**返回：** 会话 ID (UUID)

**示例：**

```python
# 本地调试
create_session(program="/path/to/program")

# 远程调试（创建会话时直接连接）
create_session(
    program="/path/to/program",
    remote_target_type="extended-remote",
    remote_host="192.168.1.100",
    remote_port=1234
)
```

#### 2. get_session

获取指定会话的信息。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

**返回：** 会话信息对象

#### 3. get_all_sessions

获取所有会话列表。

**参数：** 无

**返回：** 会话列表

#### 4. close_session

关闭指定会话。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

### 调试控制

#### 5. start_debugging

启动调试（运行程序）。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

#### 6. stop_debugging

停止调试（中断程序）。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

#### 7. continue_execution

继续执行。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

#### 8. step_execution

单步执行（进入函数）。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

#### 9. next_execution

单步执行（跳过函数）。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

### 断点管理

#### 10. set_breakpoint

设置断点。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |
| `file` | String | 是 | 源文件路径 |
| `line` | usize | 是 | 行号 |

**返回：** 断点信息

#### 11. get_breakpoints

获取所有断点列表。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

**返回：** 断点列表

#### 12. delete_breakpoint

删除断点。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |
| `breakpoints` | Vec<String> | 是 | 断点编号列表 |

### 栈帧与变量

#### 13. get_stack_frames

获取栈帧列表。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

**返回：** 栈帧列表

#### 14. get_local_variables

获取局部变量。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |
| `frame_id` | usize | 否 | 栈帧 ID（默认为当前帧） |

**返回：** 变量列表

### 寄存器

#### 15. get_registers

获取寄存器值。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |
| `reg_list` | Vec<String> | 否 | 寄存器编号列表（默认全部） |

**返回：** 寄存器列表

#### 16. get_register_names

获取寄存器名称。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |
| `reg_list` | Vec<String> | 否 | 寄存器编号列表（默认全部） |

**返回：** 寄存器名称列表

### 内存操作

#### 17. read_memory

读取内存内容。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |
| `offset` | isize | 否 | 偏移量 |
| `address` | String | 是 | 内存地址 |
| `count` | usize | 是 | 读取字节数 |

**返回：** 内存内容列表

### 远程调试

#### 18. connect_remote

连接到远程 gdbserver。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |
| `target_type` | String | 是 | 目标类型：`remote` 或 `extended-remote` |
| `host` | String | 是 | gdbserver 主机地址 |
| `port` | u16 | 是 | gdbserver 端口号 |

**示例：**

```python
connect_remote(
    session_id="xxx",
    target_type="extended-remote",
    host="192.168.1.100",
    port=1234
)
```

#### 19. disconnect_remote

断开远程连接。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |

#### 20. load_symbols

加载符号文件。

**参数：**

| 参数 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `session_id` | String | 是 | 会话 ID |
| `file` | String | 是 | 可执行文件路径 |

## 使用示例

### 示例 1：本地调试工作流

```python
# 1. 创建会话
session_id = create_session(program="/path/to/my_program")

# 2. 设置断点
set_breakpoint(session_id=session_id, file="main.c", line=42)

# 3. 启动调试
start_debugging(session_id=session_id)

# 4. 查看栈帧
frames = get_stack_frames(session_id=session_id)

# 5. 查看局部变量
vars = get_local_variables(session_id=session_id)

# 6. 单步执行
step_execution(session_id=session_id)

# 7. 继续执行
continue_execution(session_id=session_id)

# 8. 关闭会话
close_session(session_id=session_id)
```

### 示例 2：远程调试工作流

```python
# 1. 在目标机器上启动 gdbserver
# gdbserver :1234 /path/to/program

# 2. 创建会话（不指定 program，稍后加载）
session_id = create_session()

# 3. 连接到远程目标
connect_remote(
    session_id=session_id,
    target_type="extended-remote",
    host="192.168.1.100",
    port=1234
)

# 4. 加载符号文件
load_symbols(session_id=session_id, file="/path/to/local/program")

# 5. 设置断点
set_breakpoint(session_id=session_id, file="main.c", line=100)

# 6. 继续执行
continue_execution(session_id=session_id)

# 7. 查看寄存器
registers = get_registers(session_id=session_id)

# 8. 断开远程连接
disconnect_remote(session_id=session_id)

# 9. 关闭会话
close_session(session_id=session_id)
```

## 会话状态

| 状态 | 说明 |
|------|------|
| `Created` | 会话已创建，等待调试开始 |
| `Running` | 程序正在运行 |
| `Stopped` | 程序已停止（断点/中断） |

## 配置文件

配置文件 `.env` 示例：

```env
# 服务器配置
SERVER_IP=0.0.0.0
SERVER_PORT=8080

# GDB 配置
COMMAND_TIMEOUT=30
```

## 日志

日志文件存储在 `logs/` 目录下，按日期滚动生成。日志级别可通过 `--log-level` 参数设置。

## 传输模式

### Stdio 模式

标准输入输出模式，适用于管道通信。

```bash
./mcp_server_gdb --transport stdio
```

### SSE 模式

Server-Sent Events 模式，支持网络连接。

```bash
./mcp_server_gdb --transport sse
```

## TUI 界面

启用 TUI 界面需要使用 SSE 传输模式：

```bash
./mcp_server_gdb --transport sse --enable-tui
```

**快捷键：**

| 按键 | 功能 |
|------|------|
| Tab | 切换显示模式 |
| F1 | 显示全部 |
| F2 | 只显示寄存器 |
| F3 | 只显示栈 |
| F4 | 只显示反汇编 |
| F5 | 只显示输出 |
| F6 | 只显示内存映射 |
| F7 | 只显示十六进制dump |
| q | 退出 TUI |

## 常见问题

### Q: 远程调试连接失败

**可能原因：**
- gdbserver 未启动
- 防火墙阻止了端口
- 目标地址或端口错误

**解决方法：**
1. 确保 gdbserver 在目标机器上运行：`gdbserver :1234 program`
2. 检查防火墙设置，允许指定端口的入站连接
3. 验证目标地址和端口是否正确

### Q: 符号加载失败

**可能原因：**
- 符号文件路径不正确
- 可执行文件与符号文件不匹配

**解决方法：**
1. 确保指定的符号文件路径正确
2. 使用与目标程序完全相同的可执行文件作为符号文件

### Q: 断点设置失败

**可能原因：**
- 源文件路径不正确
- 行号超出范围
- 程序未停止

**解决方法：**
1. 确认源文件路径正确
2. 确认行号在有效范围内
3. 在设置断点前确保程序已停止

## 技术支持

如有问题，请查看项目仓库或提交 issue。
