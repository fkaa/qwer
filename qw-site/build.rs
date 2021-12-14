use std::path::Path;

fn main() {
    let watch_paths = qw_doc_gen::generate_help_directory(
        Path::new("./docs/tree.json"),
        Path::new("./docs/output"),
    )
    .unwrap();

    for p in watch_paths {
        println!("cargo:rerun-if-changed={}", p);
    }
}
