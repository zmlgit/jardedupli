
use clap::Parser;
use std::io::Write;
mod handler;
mod maven_coordinate;#[derive(Parser,Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// 源lib文件路径
    #[arg(short, long)]
    source: String,
    /// 目标lib文件路径
    #[arg(short, long)]
    target: String,
    /// 合并结果路径
    #[arg(short, long)]
    result: Option<String>,
    /// 仲裁文件路径
    #[arg(short, long)]
    whitelist: Option<String>,

    /// 合并策略 source:以源lib为主 target:以目标lib为主
    #[arg(long)]
    strategy: Option<String>,
}

fn main() {
    let args = Cli::parse();

    if args.source.is_empty() || args.target.is_empty() {
        println!("请提供源lib和目标lib文件路径");
        std::process::exit(1);
    }
    let management = if let Some(management_file) = args.whitelist {
        if std::path::Path::new(&management_file).exists() {
            Some(
                handler::read_management_file(management_file.as_str()).unwrap_or_else(|e| {
                    eprintln!("Error reading management file: {}", e);
                    std::process::exit(-1);
                }),
            )
        } else {
            None
        }
    } else {
        None
    };

    let sources = maven_coordinate::MavenCoordinates::read_from_path(args.source.as_str())
        .unwrap_or_else(|e| {
            eprintln!("Error reading source path: {}", e);
            std::process::exit(-1);
        });
    if !std::path::Path::new(&args.target).exists() {
        if let Some(parent) = std::path::Path::new(&args.target).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    let targets = maven_coordinate::MavenCoordinates::read_from_path(args.target.as_str())
        .unwrap_or_else(|e| {
            eprintln!("Error reading target path: {}", e);
            std::process::exit(-1);
        });
    let strategy = if let Some(strategy) = args.strategy {
        match strategy.as_str() {
            "source" | "target" => strategy,
            _ => {
                println!("Invalid strategy. Use 'source' or 'target'.");
                std::process::exit(1);
            }
        }
    } else {
        "target".to_string()
    };

    let merge_result = if strategy == "source" {
        handler::merge_jars(sources, targets, management).unwrap_or_else(|e| {
            eprintln!("Error merging jars: {}", e);
            std::process::exit(-1);
        })
    } else {
        handler::merge_jars(targets, sources, management).unwrap_or_else(|e| {
            eprintln!("Error merging jars: {}", e);
            std::process::exit(-1);
        })
    };

    let mut copied = 0u32;
    let mut skipped = 0u32;
    for (_key, coord) in &merge_result {
        let artifact_name = format!(
            "{}:{}",
            coord.group_id.as_deref().unwrap_or("?"),
            coord.artifact_id
        );
        if let Some(ref jar_path) = coord.jar_path {
            if !jar_path.is_empty() && std::path::Path::new(jar_path).exists() {
                let filename = std::path::Path::new(jar_path).file_name();
                if let Some(fname) = filename {
                    let target_path = std::path::Path::new(&args.target).join(fname);
                    if let Some(parent) = target_path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let (Ok(src_canon), Ok(target_canon)) = (
                        std::path::Path::new(jar_path).canonicalize(),
                        target_path.canonicalize(),
                    ) {
                        if src_canon == target_canon {
                            println!("[SKIP] {} (already in target)", artifact_name);
                            skipped += 1;
                            continue;
                        }
                    }
                    match std::fs::copy(jar_path, &target_path) {
                        Ok(_) => {
                            println!("[COPY] {} -> {}", jar_path, target_path.display());
                            copied += 1;
                        }
                        Err(e) => {
                            eprintln!(
                                "[ERROR] copying {} -> {}: {}",
                                jar_path,
                                target_path.display(),
                                e
                            );
                        }
                    }
                }
            } else {
                println!("[SKIP] {} (jar not found: {})", artifact_name, jar_path);
                skipped += 1;
            }
        } else {
            println!("[SKIP] {} (no jar path)", artifact_name);
            skipped += 1;
        }
    }
    println!("Done: {} copied, {} skipped", copied, skipped);
    if let Some(result_path) = args.result {
        let mut result = std::fs::File::create(result_path).unwrap_or_else(|e| {
            eprintln!("Error creating result file: {}", e);
            std::process::exit(-1);
        });
        let json = serde_json::to_string_pretty(&merge_result.values().collect::<Vec<_>>())
            .unwrap_or_else(|e| {
                eprintln!("Error serializing to JSON: {}", e);
                std::process::exit(-1);
            });
        result.write_all(json.as_bytes()).unwrap_or_else(|e| {
            eprintln!("Error writing to result file: {}", e);
            std::process::exit(-1);
        });
    } 
    std::process::exit(0);
}
