# Frontend Development

## Quick Start

### Build CSS

```bash
make tailwind
```

该命令会：

1. 自动下载 Tailwind CSS CLI（如果不存在）
2. 从 `templates/static/css/tailwind.input.css` 生成最终的 CSS
3. 输出到 `templates/static/css/tailwind.css`

### Auto-download

第一次运行 `make tailwind` 时会自动下载 Tailwind CSS standalone binary 到 `tools/tailwindcss`。

手动下载：

```bash
make download-tailwind
```

## 目录结构

```text
templates/
├── dashboard.html              # 主页面模板
├── partials/                   # 模板片段（未来使用）
└── static/
    ├── css/
    │   ├── tailwind.input.css  # Tailwind 源文件
    │   └── tailwind.css        # 生成的 CSS（gitignored）
    └── js/
        └── app.js              # JavaScript 逻辑

tools/
└── tailwindcss                 # Tailwind CLI binary（gitignored）
```

## Git 忽略文件

以下文件已在 `.gitignore` 中配置：

- `tools/tailwindcss` - Tailwind CSS binary
- `templates/static/css/tailwind.css` - 生成的 CSS
- `node_modules/` - Node 依赖（如果使用）
- `package-lock.json` - NPM lock 文件

## 技术栈

- **模板引擎**: Askama (Rust compile-time templates)
- **CSS**: Tailwind CSS v4
- **JS 库**: HTMX (hypermedia-driven interactions)
- **字体**: Google Fonts (Inter)
