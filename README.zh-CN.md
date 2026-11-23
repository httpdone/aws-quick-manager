[English](README.md) | [中文](README.zh-CN.md)

# AWS Quick Manager

**Amazon Quick 桌面端**的多账号工具，支持 Windows 和 macOS。非官方工具，与 Amazon Web Services 无关。

## 一键切号

Quick 桌面端本来就能在本机存多个账号，但界面里只能退出登录，不能切换。这个工具点一下就能把 Quick 切到另一个账号。切换前会先刷新目标账号的令牌，失败就不切。然后退出 Quick，备份账号清单，写入目标账号的登录信息，把 `last_active` 指向它，再重新启动 Quick。每个账号的本地数据和设备身份各自独立，切回原账号，原来的数据都还在。添加新账号也不用退出 Quick，可以用浏览器登录，也可以直接导入 Quick 里已登录的账号。

## 跨账号导入会话

Quick 的聊天记录存在云端，按账号保存。本地的 `sessions.db` 只是一份镜像，换了账号就打不开原来的会话，复制本地文件也没用。这个工具走的是 Quick 自己的云端接口。它先用 Quick 自带的后台 `quickwork-agent --outpost-only` 取得账号的 SNCS 令牌，然后按桌面端的方式调用 `invokeAgent` 接口：从源账号读出会话的原始事件，在目标账号新建一个会话，把事件逐条改写后上传，复制失败会自动删掉不完整的会话。源会话只读，不会被改动。附件和工作区文件不会一起复制。

## 其他功能

- 账号列表，卡片上直接显示剩余的 Agent 时长
- 会话导出成 Markdown 或 JSON
- 每个账号独立的设备 ID，可以更换或恢复
- 切换前自动备份账号清单，可以随时恢复
- 托盘、开机自启、浅色/深色主题
- 命令行：`aqm list`、`aqm current`、`aqm switch <序号|邮箱|id>`

不做反向代理和 API 转发，需要的话用 [magpie-aws-quick](https://github.com/httpdone/magpie-aws-quick)。

## 开发

需要 Node ≥ 22、pnpm 和 Rust。

```bash
pnpm install
pnpm tauri dev
cd src-tauri && cargo test
```

## 社区

有问题、发现 bug、有建议，或者想一起折腾？欢迎提 Issue 和 Pull Request。想找我或者和其他用户交流，最好去 **[linux.do](https://linux.do)**。

## 许可证

MIT
