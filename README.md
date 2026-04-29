# FileShare

一个基于 Tauri 的局域网文件共享工具。

管理端是本机桌面应用，客户端是局域网内可访问的 Web 页面。管理端可以共享本机文件和发布文本；客户端可以上传文件到服务端用户的 `Downloads` 目录，也可以发布文本。双方的共享列表会实时同步。

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
