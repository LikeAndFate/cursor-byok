# 手机远控前端实现

## 目录与职责

```text
apps/
├── mobile/                              # 独立 React 19 / Vite 8 PWA
│   ├── src/api/                        # 手机同源 `/api` 契约与类型
│   ├── src/commands/                   # 写命令可用性、幂等回执轮询
│   ├── src/pairing/                    # fragment 票据读取、内存凭据、确认轮询
│   ├── src/questions/                  # AskQuestion 单选、多选、自由文本转换
│   ├── src/state/                      # SSE 全量快照 seq/epoch reducer
│   ├── src/pages/                      # 配对页、会话列表和会话详情
│   └── vite.config.ts                  # `/mobile/` base/scope，只预缓存静态资源
└── desktop/
    ├── src/features/settings/RemoteControlSettingsCard.tsx
    │                                      # 本机 `/__byok-api__/api/remote` 管理界面
    ├── scripts/build-bundle.mjs           # desktop → mobile → dist/mobile 构建顺序
    └── src-tauri/src/mobile_frontend.rs   # 只读嵌入式 `mobile/*` 静态资源路由
```

## 主执行路径

```text
桌面生成五分钟 ticket URL/QR
  → 手机从 hash fragment 读取 ticket
  → 立即 replaceState 删除 URL 中 ticket，仅 React 内存保留
  → redeem 得到 pending id/secret（仅内存）
  → 电脑确认
  → 手机轮询 complete，成功后由服务端写 Secure/HttpOnly/SameSite Cookie
  → status 获取 CSRF 和进程 epoch
  → sessions → snapshot → SSE snapshots
  → reducer 按 epoch 重置、按 seq 去重
  → append / stop / answer_question 带 command_id、target_ref、CSRF
  → accepted 回执只轮询 GET，写请求从不自动重放
```

## 已实现行为

- `HashRouter`；PWA 的 base、start URL 和 Service Worker scope 均为 `/mobile/`。
- Workbox 只预缓存构建静态资源；无 API、认证或会话 runtime cache，并明确不承诺锁屏通知。
- 系统语言默认：中文及不支持语言使用简体中文，英语和葡萄牙语分别使用 `en-US`、`pt-BR`。
- 会话正文、工具和终端输出通过 React 文本节点及 `<pre>` 呈现，不解析 HTML；截断状态可见。
- 自动滚动仅在用户原本位于底部附近时继续跟随。
- `online`、页面可见性恢复时重取快照；SSE reset 触发重取。
- 离线、无 `target_ref`、不可控制或 `synchronized=false` 时禁用所有写操作。
- AskQuestion 支持单选、多选和每题自由文本。
- 桌面卡片支持默认关闭、43127 端口、HTTPS 来源、工作区授权、二维码、待确认设备、撤销设备和只展示 Tailscale Serve 命令。
- `build:bundle` 保持 Tauri `frontendDist` 为 `../dist`，把独立手机产物复制到 `desktop/dist/mobile`。
- Rust 路由只读取嵌入资源的 `mobile/*`，拒绝父目录、根目录和 Windows 前缀路径，不回退桌面 index。

## 测试和验证

- 手机测试：12 个测试文件、17 个测试，覆盖 reducer 重复/旧 seq/新 epoch、离线与断线禁写、SSE 销毁取消重连、问题答案互斥、16 KiB 输入限制、危险 HTML、静态缓存排除、配对未确认、票据清除、回执轮询、靠底部判断。
- `apps/mobile npm run check`：通过（typecheck、test、Vite/PWA build）。
- `apps/desktop npm run check`：通过（桌面与 Node 类型检查、desktop/mobile 联合生产构建）。
- `cargo fmt --all`：通过。
- `cargo clippy --workspace --all-targets -- -D warnings`：通过。
- `cargo test --workspace`：通过；本次验证中所有测试组均为 0 失败。
- 额外修复了 Windows checkout 的 CRLF 对内嵌运行时模板的影响：加载 Prompt 资源时统一规范化为 LF，保证跨平台前缀缓存与模板断言一致。
- 额外验证了远控 guard 不覆盖手机哈希静态资源的 immutable 缓存头；API 仍保持 `no-store`。

## 接线要求与风险

- 父进程需在 `src-tauri/src/lib.rs` 声明 `mod mobile_frontend;`，并在桌面 Axum 路由中安装 `mobile_frontend::router(app)`；当前实现已经完成这两个接线点。
- `RemoteControlSettingsCard` 假定管理 API 完全遵循 `implementation-contract.md`；未请求后端契约变更。
- PWA 的离线能力仅限应用壳静态资源，认证、会话和命令始终要求在线。
