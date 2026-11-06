fn main() {
    if std::env::args().any(|arg| arg == "--schema") {
        println!("{}", serde_json::to_string_pretty(&vork_scene::v2::schema().to_value()).unwrap());
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for (path, text) in vork_scene::v2::documents() {
        if std::env::args().any(|arg| arg == "--check") {
            assert_eq!(
                std::fs::read_to_string(root.join(path)).unwrap(),
                text,
                "{path} is stale; run scripts/generate-docs"
            );
        } else {
            std::fs::write(root.join(path), text).unwrap();
        }
    }
}
