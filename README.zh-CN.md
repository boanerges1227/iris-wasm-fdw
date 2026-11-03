# iris-wasm-fdw

[English](README.md) | [简体中文](README.zh-CN.md)

通过 WebAssembly 外部数据包装器，在 Supabase/Postgres 中查询 InterSystems IRIS。组件通过 IRIS Atelier HTTP/JSON API 发送静态 SQL，将结果映射为 PostgreSQL 外部表。

```text
Postgres 外部表 → Supabase Wrappers → Wasm FDW → IRIS Atelier API
```

## 功能

- HTTP(S) Basic Auth 认证。
- 整数、浮点、布尔、文本、日期、时间戳、带时区时间和 JSON 类型映射。
- HTTP、IRIS SQL、字段类型及数值越界错误的明确反馈。
- 固定 Rust 工具链和依赖锁文件，生成 release Wasm 组件。

当前适用于返回数据量可控的只读查询和小规模试用。

## 环境要求

- IRIS 已启用 Atelier API，查询账号有目标命名空间的访问权限。
- PostgreSQL 安装支持 Wasm 的 Supabase Wrappers 扩展。
- PostgreSQL 所在主机或容器能够访问 IRIS API。
- 使用 rustup 安装 Rust，构建工具为 cargo-component 0.13.2。

本地已验证 IRIS 2026.2、PostgreSQL 17、Wrappers 0.6.1；其他版本需要单独验证兼容性。
上述验证使用本地容器，尚未在 Supabase 托管云项目中验证。

## 构建

克隆或下载项目后，在根目录执行：

```sh
source "$HOME/.cargo/env"
bash scripts/setup-rust.sh
bash build.sh
```

`rust-toolchain.toml` 固定 Rust 1.82.0 和 `wasm32-unknown-unknown` 目标。初始化脚本使用锁定依赖安装 cargo-component 0.13.2。上传项目时保留 `Cargo.lock`。

产物路径：

```text
target/wasm32-unknown-unknown/release/iris_fdw.wasm
```

对实际分发的文件计算 SHA256：

```sh
# macOS
shasum -a 256 target/wasm32-unknown-unknown/release/iris_fdw.wasm
# Linux
sha256sum target/wasm32-unknown-unknown/release/iris_fdw.wasm
```

`src/bindings.rs` 根据 WIT 在构建时生成，不纳入版本控制。

## 配置 PostgreSQL

将 Wasm 上传到 PostgreSQL 可访问的地址，修改 [examples/setup.sql](examples/setup.sql) 中的包地址、校验和、API 地址及账号等占位符。已有包装器时复用现有扩展 schema 和名称，不重复创建。

```sql
CREATE EXTENSION IF NOT EXISTS wrappers;

CREATE FOREIGN DATA WRAPPER wasm_wrapper
  HANDLER wasm_fdw_handler VALIDATOR wasm_fdw_validator;

CREATE SERVER iris_server FOREIGN DATA WRAPPER wasm_wrapper OPTIONS (
  fdw_package_url 'https://YOUR_HOST/iris_fdw.wasm',
  fdw_package_name 'intersystems:iris-fdw',
  fdw_package_version '0.1.0',
  fdw_package_checksum 'REPLACE_WITH_ACTUAL_SHA256',
  api_url 'https://YOUR_IRIS_HOST/api/atelier/v3',
  namespace 'USER',
  username 'YOUR_READ_ONLY_USER',
  password 'YOUR_PASSWORD'
);

CREATE FOREIGN TABLE iris_people (name text)
  SERVER iris_server
  OPTIONS (query 'SELECT TOP 100 Name AS name FROM Sample.Person');

SELECT * FROM iris_people;
```

将 `Sample.Person` 换成实际源表。IRIS 返回字段的别名必须与外部表列名一致。若 Wrappers 安装在单独的 schema 中，需要为 handler 和 validator 加上对应 schema 前缀。

API 地址以 Atelier 版本路径结尾，组件会追加 `/<namespace>/action/query`。容器中的 `localhost` 指向容器自身。

请显式配置 `api_url`、`namespace`、`username` 和 `password`。如果省略，代码会回退到本地演示地址 `http://localhost:52773/api/atelier/v3`、命名空间 `USER` 和演示账号 `_SYSTEM` / `SYS`。明文 HTTP 上的 Basic Auth 不会加密凭据；非本地连接应使用 HTTPS。示例中的密码存放在外部服务器选项中，包装器没有实现密钥管理。

## 类型映射和边界

| PostgreSQL 类型 | 接受的源数据表示 |
| --- | --- |
| SMALLINT / INTEGER / BIGINT | JSON 整数或整数文本，检查目标位宽范围 |
| REAL / DOUBLE PRECISION / NUMERIC | JSON 数字或数值文本，拒绝非有限数与越界 |
| BOOLEAN | JSON 布尔、0/1，或小写文本 `true`、`false`、`0`、`1` |
| TEXT | 字符串，或 JSON 对象/数组序列化后的文本 |
| DATE | `YYYY-MM-DD` 或以 1840-12-31 为零点的 IRIS 内部天数 |
| TIMESTAMP / TIMESTAMPTZ | RFC3339 或 SQL 格式日期时间，可含小数秒 |
| JSON / JSONB | JSON 值或合法 JSON 文本，受宿主数值精度检查约束 |

- NUMERIC 使用当前 WIT 的 `f64` 载体，不保证任意小数精度；需要精确数值时，应让 IRIS 返回字符串并映射为 TEXT，TEXT 不接受 JSON 数字标量。
- 无时区时间按 UTC 解释，有偏移时间统一换算为 UTC；时间戳精度为微秒。
- JSON null 映射为 SQL NULL。Atelier 可能将 NULL 返回为空串：非文本/JSON 空串映射为 NULL，TEXT 保留空串，JSON 空串报错。
- Wrappers 0.6.1 会再次解析 JSON。可能被宿主静默舍入的数字会明确报错并提示使用 TEXT；高精度 JSON 应以字符串、对象或数组形式返回，声明为 TEXT，再在 PostgreSQL 中转换为 `jsonb`。JSON 数字标量不能直接映射为 TEXT。
- 缺列和不兼容类型会明确报错，不静默返回 NULL。

## 查询范围

`query` 是静态 SQL，不支持占位符替换、参数绑定、自动分页或 WHERE/LIMIT/排序自动下推。请直接在源 SQL 中设置过滤条件和合适的 IRIS `TOP` 限制；响应会缓存在内存中。

不实现 INSERT、UPDATE、DELETE 和宿主驱动的重新扫描。独立重复 SELECT 可以正常执行，但依赖重新扫描的连接计划可能失败。FDW 查询入口本身不校验 SQL 是否只读，实际使用应配置具备适当只读权限的 IRIS 账号。

## 目录结构

```text
.github/workflows/       公开构建、静态检查及发布工作流
examples/               配置 SQL 和发布模板
scripts/                工具链初始化及公开检查入口
src/                    FDW 实现与类型转换
supabase-wrappers-wit/   宿主接口定义
wit/                    组件接口
.local/                 本地测试、过程文档及工作日志（Git 排除）
```

执行 `bash scripts/check.sh` 完成构建、格式及 Clippy 检查。项目中的 CI 工作流配置为执行这些检查并上传构建产物；发布工作流配置为对匹配版本的标签使用该产物并计算校验和。这些工作流尚未在远程仓库验证。

## 许可证

[MIT License](LICENSE)。
