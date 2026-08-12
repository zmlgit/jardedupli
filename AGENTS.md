# jardedupli

JAR 包拷贝去重 CLI。Rust 2024 edition。Jenkins 流水线用来把多模块构建物合并到部署目录，按 maven 坐标去重 + whitelist 版本仲裁，保留新构建的 SNAPSHOT。

## 结构

```
src/
├── main.rs              CLI + run() 编排
├── lock.rs              FileLock（RAII 文件锁，防并发损坏 target）
├── merge.rs             Strategy enum + merge_jars（合并逻辑）
└── maven_coordinate.rs  MavenCoordinates + IO（读 jar/dir/json）+ 文件名解析
```

## 业务规则（关键）

### Whitelist 仲裁语义（CRITICAL）

`merge_jars` 对每个 key（`groupId:artifactId` 或 `artifactId`）的处理，**whitelist 优先**：

| whitelist 状态 | 候选版本状态 | 处理 |
|---|---|---|
| 定义了 key | whitelist 版本在候选（source/target） | 只拷贝 whitelist 版本，其余版本丢弃 |
| 定义了 key | whitelist 版本不在候选 | 跳过该 key（不拷贝，相当于屏蔽） |
| 未定义 key | 同 key 多版本冲突 | 按 strategy 选 primary（source=新构建 / target=旧部署） |
| 未定义 key | 同 key 同版本 | 按 strategy 选 primary |
| 未定义 key | 只在 source | 用 source |
| 未定义 key | 只在 target | 用 target（保留部署） |

**whitelist 命中但版本不存在 = 屏蔽该包**（`[BLOCK]` 日志 + 跳过，不再 fallback 到 strategy）。

**classifier 不参与 key**，**无 groupId 时 key 仅 artifactId**。

### SNAPSHOT 必须能更新（这是项目的核心约束）

Maven `*-SNAPSHOT` jar pom.properties version 字符串不变但内容会重新构建。
默认 `--strategy source`（`Strategy::Source`）保证新构建物覆盖旧部署。
**永远不要把默认改成 target**——会让 merge_jars 把 target 自己当 primary，触发 main.rs 的 canonicalize 相同 SKIP，SNAPSHOT 永远不更新。

## 不要做的（反模式）

- **不要在 main.rs 用 `process::exit()` 跳过函数返回** —— `_lock: FileLock` 的 Drop 在函数 return 时跑；提前 exit 不释放锁文件
- **不要把 `Strategy` 改回 `&str`** —— 类型穿透是为了防止 typo（"taret" vs "target"）静默 fallback
- **不要拆 `maven_coordinate.rs`** —— 253 行业务代码，拆完每个文件 80 行是过度细分
- **不要引入 trait / interface** —— 只有一种实现，YAGNI
- **不要把 byte-index slice 换回 char-index** —— `extract_artifact_id` / `split_artifact_version` 用 `char_indices` 是为了多字节（中文）文件名不 panic
- **不要删 `read_from_json_file` 的路径包含在错误信息里** —— 调试时定位文件用

## 关键设计

### lock.rs

RAII 文件锁。`FileLock::acquire(dir, timeout)` 用 `OpenOptions::create_new` 原子创建 `<target>/.jardedupli.lock`，60s 内轮询 500ms；Drop 时删锁。
锁文件写 PID 帮助排查 stale lock。

### main.rs

`run(args: Cli) -> i32` 持有 `_lock` 到函数末尾。main 调用 run 后 `process::exit(code)`，此时 Drop 已跑完。

### merge.rs

`Strategy` 是 `clap::ValueEnum`，main.rs Cli 直接用 `Strategy` 类型字段，clap 自动校验非法值。
`merge_jars` 返回 `BTreeMap` 保证日志顺序稳定（按 key 字典序）。

### maven_coordinate.rs

- `read_from_path(&[String])` — glob 展开多个 source 路径，递归读 jar
- `read_from_json_file(path)` — 读 whitelist JSON
- `from_jar(path)` — 读 META-INF/maven/*/pom.properties；多个时按文件名启发式挑主坐标；找不到时按文件名推测（找第一个 `-数字` 切分）
- `shellexpand_home(p)` — 展开 `~/`，glob 自身不识别

## 命令

```bash
# 构建（默认 host target）
cargo build --release

# 构建 musl 静态二进制（部署到 Alpine 等需要）
cargo build --release --target x86_64-unknown-linux-musl

# 测试（28 个用例）
cargo test --release

# Jenkins 部署典型用法
jardedupli -s /jenkins/workspace/cms-fsmc/lib -t /data/backend/cms-fsmc/app/lib -w lib.json

# 多源合并
jaredupli -s /jenkins/build-a/lib -s /jenkins/build-b/lib -t /data/deploy/lib -w lib.json

# 输出合并结果 JSON（用于审计）
jardedupli -s src/ -t target/ -r result.json -w lib.json

# 保留现有部署不更新（少见）
jardedupli -s src/ -t target/ --strategy target
```

## 日志格式

| 输出 | 含义 |
|---|---|
| `[COPY] <src> -> <dest>` | 拷贝成功 |
| `[SKIP] <name> (already in target)` | 源/目标是同一文件（canonicalize 相同） |
| `[SKIP] <name> (jar not found: <path>)` | jar_path 不存在 |
| `[SKIP] <name> (no jar path)` | MavenCoordinates 无 jar_path 字段 |
| `[ERROR] ...` | 失败，stderr |
| `[WARN] ...` | 非致命警告（whitelist 不存在等），stderr |
| `[BLOCK] <key> (excluded by whitelist version '<v>')` | whitelist 命中但版本不在候选 → 屏蔽（不部署），stdout |
| `Done: N copied, M skipped, K errors` | 总结 |

**exit code**：0 成功 / 1 业务错误（含 copy 失败、source 为空、lock 失败等）/ 2 clap 参数错误。

## 历史踩坑

| 现象 | 根因 | 修复 |
|---|---|---|
| SNAPSHOT 永远不更新（`[SKIP] already in target`） | 默认 strategy=target → `merge_jars(targets, sources, ...)` 反置参数 → `candidates.first()` 取到 target → jar_path=target → canonicalize 相同 → SKIP | 默认改 source；调用不反置；strategy 用 enum 穿透 |
| 多进程并发部署 jar 损坏 | `std::fs::copy` 非原子，并发覆盖撕裂文件 | 加 RAII 文件锁 |
| Jenkins 误报部署成功 | copy 失败只 eprintln，最后 `exit(0)` | errors 计数，return 1 |
| 锁文件不清理 | `process::exit` 跳过 Drop | 拆 `run()` 返回 i32，main 转 exit |
| 中文 jar 名 panic | `chars[i]` 与 `file_name[byte..]` 混用 | `char_indices` 字节安全 |
