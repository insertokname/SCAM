use clap::Parser;
use scamm::{InesHeader, create_mapper};
use serde::Serialize;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the directory containing the ROMs
    #[arg(short, long)]
    roms_dir: PathBuf,

    /// Path to the output JSON manifest file
    #[arg(short, long)]
    output: PathBuf,
}

#[derive(Serialize)]
struct RomEntry {
    name: String,
    path: String,
    category: String,
    supported: bool,
}

fn main() {
    let args = Args::parse();

    let roms_dir = args.roms_dir;
    let output_file = args.output;

    if !roms_dir.exists() || !roms_dir.is_dir() {
        eprintln!(
            "Error: ROMs directory does not exist or is not a directory: {}",
            roms_dir.display()
        );
        std::process::exit(1);
    }

    if let Some(parent) = output_file.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).unwrap_or_else(|err| {
                eprintln!(
                    "Failed to create output directory {}: {}",
                    parent.display(),
                    err
                );
                std::process::exit(1);
            });
        }
    }

    let mut entries = Vec::new();
    let mut total_supported = 0;
    let mut total_unsupported = 0;

    for entry in WalkDir::new(&roms_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.path().is_file() {
            if let Some(ext) = entry.path().extension() {
                if ext == "nes" {
                    let rel_path_obj = entry.path().strip_prefix(&roms_dir).unwrap();
                    let rel_path = rel_path_obj.to_string_lossy().replace('\\', "/");
                    let file_name = entry.file_name().to_string_lossy().to_string();

                    let category = rel_path.split('/').next().unwrap_or("Unknown").to_string();

                    let supported = check_supported(entry.path());

                    if supported {
                        total_supported += 1;
                    } else {
                        total_unsupported += 1;
                    }

                    let name = file_name
                        .strip_suffix(".nes")
                        .unwrap_or(&file_name)
                        .to_string();

                    entries.push(RomEntry {
                        name,
                        path: rel_path,
                        category,
                        supported,
                    });
                }
            }
        }
    }

    entries.sort_by(|a, b| b.supported.cmp(&a.supported).then(a.name.cmp(&b.name)));

    let json = serde_json::to_string(&entries).unwrap();
    std::fs::write(&output_file, json).expect("Failed to write roms.json");

    println!("Manifest generated at {}", output_file.display());
    println!("Supported ROMs: {}", total_supported);
    println!("Unsupported ROMs: {}", total_unsupported);
}

fn check_supported(path: &Path) -> bool {
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let mut header_bytes = [0u8; 16];
    if file.read_exact(&mut header_bytes).is_err() {
        return false;
    }

    let header = match InesHeader::parse(&header_bytes) {
        Ok(h) => h,
        Err(_) => return false,
    };

    create_mapper(header.into_config()).is_ok()
}
