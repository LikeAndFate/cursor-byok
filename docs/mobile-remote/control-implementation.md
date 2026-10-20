# 阶段 A：远程控制实现记录

## 已实现

```text
server/src/cursor/
├── conversation/remote.rs       固定远控类型与结果
├── conversation/command.rs      独立 RemoteState / RemoteControl actor 命令
├── conversation/runtime.rs      以 Run generation + 实际 RunId 校验目标
├── conversation/output.rs       定向 BreakMessages、工具中断与 Cursor UI 更新
├── compile/break_messages.rs    actor 到 output 的远程追加动作
├── tools/runtime.rs             普通 AskQuestion 快照、原子校验与消费
├── tools/mod.rs                 复用现有 AskQuestion ToolCompletion 编码
└── transport/handle.rs          remote_state / remote_command 公共入口
```

- `AppendText` 不进入 Bidi `OrderedInbox`，不创建或占用 Cursor seqno。
- 追加仅投递给捕获的当前 `RunHandle`；不经过 `ConversationRegistry::deliver`，因此不会写入 `registry.pending`，也不会跨 Run 暂存。
- `Applied` 在 `RunHandle::break_messages` 返回提交结果后才回执；actor 本身不等待提交，避免阻塞后续 exec/KV 命令。
- 追加沿用已有运行时用户消息编译、工具中断、checkpoint 和 UI `queued/delivered/user_message_appended` 路径。
- `Stop` 沿用先取消 Run、再发送正在执行工具 abort 的顺序。
- `AnswerQuestion` 仅访问普通 `AskQuestion` pending interaction；验证问题 ID、选项 ID、单选/多选和 freeform 互斥后才原子消费。非法输入不消费；桌面与手机响应由同一 pending map 实现 first-valid-wins；权限审批未开放。
- `RemoteTarget.generation` 使用 transport actor 的 `RunGeneration.id`，并同时校验实际内部 `RunId`。

## 测试

新增 `server/tests/remote_control.rs`，覆盖：

- 活动 Run state/target 与 stale target 拒绝；
- append 提交、同一 Run 的 Cursor UI 更新和 command_id 去重；
- 替换 Run 后旧 target 不影响新 Run；
- AskQuestion 非法输入不消费、有效响应后重复响应拒绝；
- Stop 先发工具 abort，再输出取消终止帧；
- Run 结束或 finalizing 后追加被拒绝，且不会进入后续 Run。

初次失败的实证根因：

- 测试 helper 丢弃了 `TempDir`，SQLite 文件在 Run prepare 前已被删除，导致 provider 永远不启动；
- 测试没有持续处理 Cursor KV 请求，或重复使用固定 seqno，导致 prepare/checkpoint 流程等待；
- append 测试同步等待提交回执却未同时消费并确认 checkpoint KV，形成测试侧死锁；
- 内部 `RunId` 带随机后缀，断言已改为验证请求前缀与 generation；
- 所有轮询和异步等待现均有 5 秒上限。

TDD 红灯已执行：

```text
cargo test -p cursor-server --test remote_control
结果：按预期编译失败，缺少 Remote* 类型和 TransportHandle 方法。
```

最终验证：

```text
cargo test -p cursor-server --test remote_control -- --nocapture
结果：6 passed；0 failed；0 ignored。

cargo test -p cursor-server --test interrupt --test cursor_transport_lifecycle
结果：interrupt 18 passed；cursor_transport_lifecycle 3 passed。
```

`prefix_stability` 未重跑；已知失败位于不属于本任务范围的 runtime 模板 CRLF `contains` 断言。未进行真实 Cursor 客户端或手机验证。

## 风险

- 测试期间父进程并行编辑 HTTP/投影模块，曾短暂造成编译错误和可执行文件锁；最终上述目标测试已在可编译状态通过。
- 真实 Cursor 与手机端到端行为仍需后续人工或系统级验证。
