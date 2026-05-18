use clap::Parser;
use std::collections::HashSet;
use std::io::Write;
mod handler;
mod maven_coordinate;
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    /// 源lib文件路径
    #[arg(short, long,num_args = 1..)]
    source: Vec<String>,
    /// 目标lib文件路径
    #[arg(short, long)]
    target: String,
    /// 合并结果路径
    #[arg(short, long)]
    result: Option<String>,
    /// 仲裁文件路径
    #[arg(short, long)]
    whitelist: Option<String>,
}

fn main() {
    let args = Cli::parse();

    if args.source.is_empty() || args.target.is_empty() {
        println!("请提供源lib和目标lib文件路径");
        std::process::exit(1);
    }

    let sources =
        maven_coordinate::MavenCoordinates::read_from_path(&args.source).unwrap_or_else(|e| {
            eprintln!("Error reading source path: {}", e);
            std::process::exit(-1);
        });
    if !std::path::Path::new(&args.target).exists() {
        let _ = std::fs::create_dir_all(std::path::Path::new(&args.target).parent().unwrap());
    }
    let merge_lock = std::path::Path::new(&args.target).join("merge.lock");
    for _ in 0..1000 {
        if merge_lock.exists() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        } else {
            break;
        }
    }
    if merge_lock.exists() {
        eprintln!("合并锁文件已存在，请检查是否有其他进程在使用该文件");
        std::process::exit(-1);
    }
    let lock_file = std::fs::File::create(&merge_lock).unwrap_or_else(|e| {
        eprintln!("Error creating lock file: {}", e);
        std::process::exit(-1);
    });
    let management = if let Some(management_file) = args.whitelist {
        Some(
            handler::read_management_file(management_file.as_str()).unwrap_or_else(
                |e: anyhow::Error| {
                    eprintln!("Error reading management file: {}", e);
                    Vec::new()
                },
            ),
        )
    } else {
        None
    };

    let merge_result = handler::merge_jars(sources, management).unwrap_or_else(|e| {
        eprintln!("Error merging jars: {}", e);
        std::process::exit(-1);
    });

    let source = maven_coordinate::get_paths(&args.source).unwrap_or_else(|e| {
        eprintln!("Error getting paths: {}", e);
        std::process::exit(-1);
    });

    let mut copied = HashSet::new();
    let target_path = std::path::Path::new(&args.target);
    if !target_path.exists() {
        let _ = std::fs::create_dir_all(target_path);
    }
    for source_jar in merge_result {
        if let Some(jar_path) = source_jar.jar_path.clone() {
            for path in jar_path.clone() {
                let jar_name = std::path::Path::new(&path).file_name().unwrap_or_default();
                let target_path = std::path::Path::new(&args.target).join(jar_name);
                if copied.contains(&path) {
                    continue;
                }
                copied.insert(path.clone());
                std::fs::copy(&path, &target_path).unwrap_or_else(|e: std::io::Error| {
                    eprintln!(
                        "Error copying file: {} -> {} : {}",
                        &path,
                        &target_path.display(),
                        e
                    );

                    std::fs::remove_file(&merge_lock).unwrap_or_else(|e| {
                        eprintln!("Error removing lock file: {}", e);
                        std::process::exit(-1);
                    });
                    std::process::exit(-1);
                });
            }
        }
    }
    for path in source {
        if copied.contains(&path) {
            println!("copied {}", path);
        } else {
            println!("not copied {}", path);
        }
    }
    if let Some(result_path) = args.result {
        let mut result = std::fs::File::create(result_path).unwrap_or_else(|e| {
            eprintln!("Error creating result file: {}", e);
            std::fs::remove_file(&merge_lock).unwrap_or_else(|e| {
                eprintln!("Error removing lock file: {}", e);
                std::process::exit(-1);
            });
            std::process::exit(-1);
        });
        let copy_result =
            maven_coordinate::MavenCoordinates::read_from_path(&vec![args.target.clone()])
                .unwrap_or_else(|e| {
                    eprintln!("Error reading target path: {}", e);
                    std::fs::remove_file(&merge_lock).unwrap_or_else(|e| {
                        eprintln!("Error removing lock file: {}", e);
                        std::process::exit(-1);
                    });
                    std::process::exit(-1);
                });

        let json =
            serde_json::to_string_pretty(&copy_result).unwrap_or_else(|e: serde_json::Error| {
                eprintln!("Error serializing to JSON: {}", e);
                std::fs::remove_file(&merge_lock).unwrap_or_else(|e| {
                    eprintln!("Error removing lock file: {}", e);
                    std::process::exit(-1);
                });
                std::process::exit(-1);
            });
        result.write_all(json.as_bytes()).unwrap_or_else(|e| {
            eprintln!("Error writing to result file: {}", e);
            std::fs::remove_file(&merge_lock).unwrap_or_else(|e| {
                eprintln!("Error removing lock file: {}", e);
                std::process::exit(-1);
            });
            std::process::exit(-1);
        });
    }
    std::fs::remove_file(&merge_lock).unwrap_or_else(|e| {
        eprintln!("Error removing lock file: {}", e);
        std::process::exit(-1);
    });
    std::process::exit(0);
}
