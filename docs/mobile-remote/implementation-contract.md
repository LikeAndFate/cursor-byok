# 手机远控实现接口（开发约定）

已批准首版：单用户 Windows、仅本项目 BYOK、Tailscale Serve 私网、PWA；只控制当前活动运行。新增任务/结束后续聊/权限审批排除。当前分支 feat/mobile-remote，不提交/推送。

## 手机 HTTP API

同源 /api；默认 Cookie 认证（Secure、HttpOnly、SameSite=Strict）。除配对页和静态资源外都需认证。所有 POST 含 Origin；命令 POST 另含 x-csrf-token（来自 status）。响应一律 no-store，失败 JSON {error:string}。

- GET /api/status → {device_id,csrf_token,epoch,host_name,version}
- POST /api/pairings/redeem {ticket,device_name} → {id,secret}，五分钟待电脑确认
- POST /api/pairings/complete {id,secret} → 202 {pending:true} 或 200 {paired:true} + Cookie；一次成功后凭据不可再次兑换
- GET /api/sessions → {sessions:SessionSummary[]}，limit<=50，before 可选分页游标
- GET /api/sessions/:id/snapshot → SessionSnapshot
- GET /api/sessions/:id/events?after=seq → SSE，每个 data 是 SessionSnapshot 全量受限展示快照；event: snapshot，id: seq。水位太旧 event: reset。心跳不推进seq。401/403不自动重试写命令。
- POST /api/sessions/:id/commands {command_id,target_ref,kind:'append_text',text} 或 {command_id,target_ref,kind:'stop'} 或 {command_id,target_ref,kind:'answer_question',question_ref,answers:[{question_id,selected_option_ids,freeform_text}]} → CommandReceipt
- GET /api/commands/:id → CommandReceipt

type SessionSummary = {id:string,title:string,workspaces:string[],model:string,updated_at_ms:number,status:'running'|'waiting_question'|'completed'|'cancelled'|'failed'|'unavailable',controllable:boolean};
type MessageView = {id:string,kind:'user'|'assistant'|'tool'|'notice',text:string,truncated:boolean};
type QuestionView = {ref:string,title:string,questions:[{id:string,prompt:string,allow_multiple:boolean,options:[{id,label}]}]};
type SessionSnapshot = SessionSummary & {seq:number,epoch:string,target_ref:string|null,messages:MessageView[],questions:QuestionView[],synchronized:boolean};
type CommandReceipt = {command_id:string,status:'accepted'|'applied'|'rejected'|'outcome_unknown',reason:string|null};

注：首版将单次合并后的展示状态作为 SSE 事件发送，而不向手机暴露 protobuf、CanonicalMessage 或内部 RunEvent。只发送确有变化的状态，正文/终端有长度限制并显示截断。禁止 HTML 注入。手机离线禁止操作，无离线命令重放。切后台恢复重取status/snapshot。结束后输入禁用。

## 桌面本机管理 HTTP

仍用 /__byok-api__/api，不新增 IPC。以下请求沿用现有 shared/api.ts request 风格：
- GET /remote → {settings:RemoteSettings,running:boolean,error:string|null,pairings:PendingPairing[],devices:RemoteDevice[],projects:RemoteProject[]}
- PUT /remote {enabled,port,public_origin,allowed_workspaces} → 同 GET
- POST /remote/pairings → {secret,url,expires_at_ms}；url为 public_origin + '/mobile/#/pair?ticket=...' （仅短期票据，非设备凭据）
- POST /remote/pairings/:id/confirm → {confirmed:true}
- DELETE /remote/devices/:id → {revoked:true}

type RemoteSettings = {enabled:boolean,port:number,public_origin:string,allowed_workspaces:string[]}; 默认 false/43127/''/[]。
type PendingPairing={id:string,device_name:string,expires_at_ms:number,confirmed:boolean};
type RemoteDevice={id:string,name:string,created_at_ms:number,last_seen_at_ms:number|null,revoked_at_ms:number|null};
type RemoteProject={path:string}; projects 为从实际请求捕获的所有工作区路径（本机显示可选授权），用户手动勾选，未知归属不开放。

QR 仅短期票据URL；须电脑确认。Serve command只展示，不自动执行或更改Tailscale规则。不要把完整桌面端口对外开放。

## 内部控制 API（由独立实现任务负责）
TransportHandle::remote_state()->Option<RemoteState>; remote_command(target,command)->RemoteResult。
RemoteTarget {run_id:RunId,generation:u64}; RemoteCommand::AppendText{command_id,text},Stop,AnswerQuestion{interaction_id,tool_call_id,answers}；RemoteAnswer字段与HTTP同；RemoteQuestion{interaction_id,tool_call_id,args:pb::AskQuestionArgs}; RemoteResult::Applied,Duplicate,RunEnded,RunClosing,StaleTarget,AlreadyResolved,InvalidInput。

外部层持有不透明 target_ref，绑定实际 handle+内部 target+进程 epoch；不依赖客户端提交执行ID。
