use clap::Parser;
use merge::Strategy;
use maven_coordinate::MavenCoordinates;
use std::io::Write;
use std::time::Duration;

mod lock;
mod merge;
mod maven_coordinate;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// 源 lib 路径，可多次传入或用 glob（如 /jenkins/*/lib）
    #[arg(short, long, num_args = 1.., required = true)]
    source: Vec<String>,

    /// 目标 lib 路径
    #[arg(short, long, required = true)]
    target: String,

    /// 合并结果 JSON 输出路径
    #[arg(short, long)]
    result: Option<String>,

    /// 仲裁文件路径（JSON）
    #[arg(short, long)]
    whitelist: Option<String>,

    /// 同 key 同版本时的拷贝方向：source=新构建覆盖部署（默认），target=保留现有部署
    #[arg(long, value_enum, default_value_t = Strategy::Source)]
    strategy: Strategy,
}

fn main() {
    let args = Cli::parse();
    let exit_code = run(args);
    // 注意：不要在这里 process::exit，否则 _lock 的 Drop 不会执行。
    // run() 内部已经把 _lock 持有到函数末尾，Drop 在 return 时跑完。
    std::process::exit(exit_code);
}

fn run(args: Cli) -> i32 {
    let management = match args.whitelist.as_deref() {
        Some(path) if std::path::Path::new(path).exists() => {
            match MavenCoordinates::read_from_json_file(path) {
                Ok(m) => Some(m),
                Err(e) => {
                    eprintln!("[ERROR] read whitelist {}: {}", path, e);
                    return 1;
                }
            }
        }
        Some(path) => {
            eprintln!("[WARN] whitelist not found: {}, continuing without it", path);
            None
        }
        None => None,
    };

    let target_path = std::path::Path::new(&args.target);
    if let Err(e) = std::fs::create_dir_all(target_path) {
        eprintln!("[ERROR] create target dir {}: {}", target_path.display(), e);
        return 1;
    }

    let _lock = match lock::FileLock::acquire(target_path, Duration::from_secs(LOCK_TIMEOUT_SECS)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[ERROR] acquire lock under {}: {}", target_path.display(), e);
            return 1;
        }
    };

    let sources = match MavenCoordinates::read_from_path(&args.source) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[ERROR] read source: {}", e);
            return 1;
        }
    };
    if sources.is_empty() {
        eprintln!(
            "[ERROR] no .jar found under source paths: {:?}",
            args.source
        );
        return 1;
    }
    let targets = match MavenCoordinates::read_from_path(&[args.target.clone()]) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("[ERROR] read target: {}", e);
            return 1;
        }
    };

    let merge_result = match merge::merge_jars(sources, targets, management, args.strategy) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("[ERROR] merge: {}", e);
            return 1;
        }
    };

    let mut copied: u32 = 0;
    let mut skipped: u32 = 0;
    let mut errors: u32 = 0;

    for (_key, coord) in &merge_result {
        let artifact_name = format!(
            "{}:{}",
            coord.group_id.as_deref().unwrap_or("?"),
            coord.artifact_id
        );
        let Some(jar_path) = coord.jar_path.as_deref() else {
            println!("[SKIP] {} (no jar path)", artifact_name);
            skipped += 1;
            continue;
        };
        if jar_path.is_empty() || !std::path::Path::new(jar_path).exists() {
            println!("[SKIP] {} (jar not found: {})", artifact_name, jar_path);
            skipped += 1;
            continue;
        }
        let Some(fname) = std::path::Path::new(jar_path).file_name() else {
            println!("[SKIP] {} (no file name in path)", artifact_name);
            skipped += 1;
            continue;
        };
        let dest = target_path.join(fname);
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let same_file = match (
            std::fs::canonicalize(jar_path),
            std::fs::canonicalize(&dest),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        };
        if same_file {
            println!("[SKIP] {} (already in target)", artifact_name);
            skipped += 1;
            continue;
        }

        match std::fs::copy(jar_path, &dest) {
            Ok(_) => {
                println!("[COPY] {} -> {}", jar_path, dest.display());
                copied += 1;
            }
            Err(e) => {
                eprintln!("[ERROR] copy {} -> {}: {}", jar_path, dest.display(), e);
                errors += 1;
            }
        }
    }

    println!(
        "Done: {} copied, {} skipped, {} errors",
        copied, skipped, errors
    );

    if let Some(result_path) = args.result {
        let result_vec: Vec<&MavenCoordinates> = merge_result.values().collect();
        let json = match serde_json::to_string_pretty(&result_vec) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[ERROR] serialize result: {}", e);
                return 1;
            }
        };
        let mut f = match std::fs::File::create(&result_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[ERROR] create result {}: {}", result_path, e);
                return 1;
            }
        };
        if let Err(e) = f.write_all(json.as_bytes()) {
            eprintln!("[ERROR] write result {}: {}", result_path, e);
            return 1;
        }
    }

    if errors > 0 {
        1
    } else {
        0
    }
}

const LOCK_TIMEOUT_SECS: u64 = 60;
