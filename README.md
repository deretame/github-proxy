# github-proxy

一个用 Rust 编写的 GitHub API 代理服务，专门放行如下安全路径：

`/repos/<owner>/Breeze-plugin-<name>/releases/latest`

- 运行时使用 `mimalloc` 作为全局分配器
- 支持 `musl` 静态构建
- 默认返回 CORS 头（`Access-Control-Allow-Origin: *`）
- 进程内内存缓存（TTL 1 小时，按 `path` 键控）
- 返回 `x-proxy-cache` 响应头标记缓存状态（`HIT/MISS/BYPASS`）

## 1. 环境变量

必须设置：

- `GITHUB_TOKEN`：GitHub Token（建议最小权限）

可选：

- `PORT`：监听端口，默认 `3000`

## 2. 本地运行

```bash
export GITHUB_TOKEN=ghp_xxx
cargo run
```

请求示例：

```bash
curl "http://127.0.0.1:3000/proxy?path=/repos/your-org/Breeze-plugin-demo/releases/latest"
```

## 3. musl 构建（静态）

先安装 musl 交叉编译工具（Ubuntu/Debian）：  
`sudo apt-get update && sudo apt-get install -y musl-tools`

先安装目标：

```bash
rustup target add x86_64-unknown-linux-musl
```

使用已配置好的 alias：

```bash
cargo build-musl
```

产物路径：

`target/x86_64-unknown-linux-musl/release/github-proxy`

## 4. 安全策略

服务仅允许匹配以下正则的路径转发到 `https://api.github.com`：

```regex
^/repos/[\w.-]+/Breeze-plugin-[\w.-]+/releases/latest$
```

不符合规则时返回 `403 Access Denied`。
