# Changelog

本文件记录 RemoteX 的面向用户的版本变更。

This file records user-facing changes for RemoteX.

## [0.1.1] — 2026-11-21

- 更新插件描述，增加 SFTP 管理、Docker 面板、主机监控、命令管理等功能介绍。
- 版本号升级至 0.1.1。

**Updated plugin description with SFTP management, Docker panel, host monitoring, command management and more. Version bumped to 0.1.1.**

## [0.1.0] — 2026-11-16

首个准备上架 DBX 商店的稳定版（`0.0.1-beta` 为预发布，未进入商店）。

- SFTP 上传遇到同名文件时，选择「覆盖」或「重命名」并勾选「应用到全部」后，同批已排队的文件不再逐个询问。
- 编辑器页签栏新增「关闭全部」按钮；有未保存修改时只确认一次。
- 修正插件描述中的协议列表（`serial and local shell`）。
- 发布构建固定从 release tag 构建，保证 `source` 与安装包字节可追溯。

**First stable release prepared for the DBX Store** (`0.0.1-beta` was a
pre-release and was never listed). Batch "apply to all" now answers the rest of
the queued conflict prompts; the editor gains a Close all button; the plugin
description lists local shell correctly; release builds are produced from the
release tag itself.

## [0.0.1-beta] — 2026-11-15

- **RemoteX 品牌首个版本**：该插件基于插件市场里的“终端”（[**dbx-plugin-ssh**](https://github.com/jinpy666/dbx-plugin-ssh)）进行开发。插件标识切换为 `io.dbx.remotex`，首个版本`0.0.1-beta`。
- 连接入口、工作台、文件系统贡献点、转发菜单与 Scheduler Task Provider 的标识同步更新。
- 插件数据目录、Known Hosts、录制、传输历史与 MCP 设置随新标识使用独立目录；旧版本数据不会自动迁移。
- 文档与 README 更新为 RemoteX 当前能力与仓库地址。

**First RemoteX-branded release.** The plugin id is now `io.dbx.remotex`, and the first version is`0.0.1-beta`. Contribution ids, the plugin
data directory, Known Hosts, recordings, transfer history, and MCP settings follow
the new identity; data from the previous id is not migrated automatically.
