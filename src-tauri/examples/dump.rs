//! Debug helper: parse a .dts and print the LoadResult JSON exactly as the
//! frontend receives it. Usage:
//!   cargo run --example dump -- <file.dts> [include_dir...]

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .expect("usage: dump <file.dts> [include_dir...]");
    let include_dirs: Vec<String> = args.collect();
    match miru_dt_lib::dts::parse_dts_file(std::path::Path::new(&path), &include_dirs) {
        Ok(result) => println!("{}", serde_json::to_string_pretty(&result).unwrap()),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
