# Changelog

本文件记录 RemoteX 的面向用户的版本变更。

This file records user-facing changes for RemoteX.

## [0.0.1-beta] — 2026-11-15

- **RemoteX 品牌首个版本**：该插件基于插件市场里的“终端”（[**dbx-plugin-ssh**](https://github.com/jinpy666/dbx-plugin-ssh)）进行开发。插件标识切换为 `io.dbx.remotex`，首个版本`0.0.1-beta`。
- 连接入口、工作台、文件系统贡献点、转发菜单与 Scheduler Task Provider 的标识同步更新。
- 插件数据目录、Known Hosts、录制、传输历史与 MCP 设置随新标识使用独立目录；旧版本数据不会自动迁移。
- 文档与 README 更新为 RemoteX 当前能力与仓库地址。

**First RemoteX-branded release.** The plugin id is now `io.dbx.remotex`, and the first version is`0.0.1-beta`. Contribution ids, the plugin
data directory, Known Hosts, recordings, transfer history, and MCP settings follow
the new identity; data from the previous id is not migrated automatically.
