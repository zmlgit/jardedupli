use std::collections::HashSet;

use clap::Parser;
use std::io::Write;
mod handler;
mod maven_coordinate;
#[derive(Parser,Debug)]
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
        Some(
            handler::read_management_file(management_file.as_str()).unwrap_or_else(|e| {
                eprintln!("Error reading management file: {}", e);
                std::process::exit(-1);
            }),
        )
    } else {
        None
    };

    let sources = maven_coordinate::MavenCoordinates::read_from_path(args.source.as_str())
        .unwrap_or_else(|e| {
            eprintln!("Error reading source path: {}", e);
            std::process::exit(-1);
        });
    if !std::path::Path::new(&args.target).exists() {
        let _ = std::fs::create_dir_all(std::path::Path::new(&args.target).parent().unwrap());
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

    let source = maven_coordinate::get_paths(args.source.as_str()).unwrap_or_else(|e| {
        eprintln!("Error getting paths: {}", e);
        std::process::exit(-1);
    });
    let copy_source = merge_result
        .values()
        .filter(|x| x.jar_path.is_some())
        .map(|x| x.jar_path.clone().unwrap_or_default().clone())
        .collect::<HashSet<_>>();
    for path in source {
        if copy_source.contains(&path) {
            let target_path = if args.target.ends_with("/") {
                args.target.clone() + path.split("/").last().unwrap_or_default()
            } else {
                args.target.clone() + "/" + path.split("/").last().unwrap_or_default()
            };
            let target_path = path.replace(args.source.as_str(), &target_path);
            if !std::path::Path::new(&target_path).exists() {
                let _ =
                    std::fs::create_dir_all(std::path::Path::new(&target_path).parent().unwrap());
            }
            if let Err(e) = std::fs::copy(path, target_path) {
                eprintln!("Error copying file: {}", e);
            }
        }else {
            println!("{} not in copy_source", path);
        }
    }
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
