fn main() {
    let project_dir = env!("CARGO_MANIFEST_DIR");

    volo_build::Builder::protobuf()
        .add_service(format!("{}/proto/example.proto", project_dir))
        .include_dirs(vec![format!("{}/proto", project_dir).into()])
        .filename("proto_gen.rs".into())
        .write()
        .unwrap();
}
