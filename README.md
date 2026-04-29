# FileShare

一个基于 Tauri 的局域网文件共享工具。

管理端是本机桌面应用，客户端是局域网内可访问的 Web 页面。管理端可以共享本机文件和发布文本；客户端可以上传文件到服务端用户的 `Downloads` 目录，也可以发布文本。双方的共享列表会实时同步。

## 下载

如果你只是想直接使用应用，请前往 GitHub Releases 页面下载已经打包好的安装包：

[GitHub Releases](https://github.com/tri5m/file-share/releases)

## 开发环境

- Node.js
- Rust / Cargo
- Tauri 对应平台依赖

## 安装依赖

```bash
npm install
```

## 启动开发

```bash
npm run dev
```

启动后会打开 Tauri 管理端窗口。输入端口并点击“启动服务”后，客户端才可以通过局域网地址访问。

开发模式说明：

- 当前前端静态资源位于 `public/`
- Rust 服务通过 `include_str!` 内嵌这些文件
- 修改 `public/` 下文件后，需要重启一次 `npm run dev`，最新前端资源才会重新编译进应用

## 构建打包

```bash
npm run build
```

打包完成后，产物默认会出现在 `src-tauri/target/release/bundle/` 目录下。

## 自动发布

仓库包含 GitHub Actions 发布流程。推送 `v*` 标签时，会自动构建：

- macOS Apple Silicon `.dmg`
- macOS Intel `.dmg`
- Windows x86 安装包

构建完成后，产物会上传到对应的 GitHub Release。
