use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn json(path: &Path) -> serde_json::Value {
    println!("cargo:rerun-if-changed={}", path.display());
    serde_json::from_slice(&fs::read(path).expect("read pinned tool metadata"))
        .expect("parse pinned tool metadata")
}

fn environment_tools(root: &Path, id: &str) -> String {
    let recipes = json(&root.join("language-clients/rust/environment.json"));
    let Some(recipe) = recipes.get(id) else {
        return "None".into();
    };
    let pins = json(&root.join(recipe["pins"].as_str().expect("tool pin source")));
    let mut platforms = serde_json::Map::new();
    for (name, artifact) in pins["platforms"].as_object().expect("target platforms") {
        if !name.starts_with("linux-") {
            continue;
        }
        let mut artifact = artifact.clone();
        if recipe["kind"] == "node" {
            let filename = artifact["url"]
                .as_str()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap();
            let prefix = filename
                .strip_suffix(".tar.xz")
                .or_else(|| filename.strip_suffix(".tar.gz"))
                .expect("Linux runtime archive");
            let member = format!("{prefix}/{}", artifact["program"].as_str().unwrap());
            let license = format!("{prefix}/LICENSE");
            artifact["member"] = serde_json::json!(member);
            artifact["license_member"] = serde_json::json!(license);
        }
        platforms.insert(name.clone(), artifact);
    }
    let mut spec =
        serde_json::json!({"schema":1,"id":id,"kind":recipe["kind"],"platforms":platforms});
    if recipe["kind"] == "node" {
        let lock = json(&root.join(recipe["lock"].as_str().expect("server lock source")));
        let packages = lock["packages"].as_object().expect("locked packages").iter().filter(|(path, _)| !path.is_empty()).map(|(path, package)| {
            assert!(package["link"] != true, "managed server packages cannot be links");
            serde_json::json!({"path":path,"url":package["resolved"].as_str().expect("locked source"),"integrity":package["integrity"].as_str().expect("locked integrity")})
        }).collect::<Vec<_>>();
        spec["packages"] = serde_json::json!(packages);
        spec["entry"] = recipe["entry"].clone();
    } else {
        spec["licenses"] = pins["licenses"].clone();
    }
    format!("Some({:?})", spec.to_string())
}

fn main() {
    println!("cargo:rustc-check-cfg=cfg(feature, values(\"environment\"))");
    println!("cargo:rerun-if-env-changed=LOOG_SOURCE");
    let source = PathBuf::from(
        env::var_os("LOOG_SOURCE").expect("set LOOG_SOURCE to the matching Loog checkout"),
    );
    let wit = source.join("wit/loog-extension");
    println!("cargo:rerun-if-changed={}", wit.display());
    let crate_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let manifest = crate_dir.join("../manifest.json");
    let configuration = crate_dir.join("../../../language-clients/rust/clients.json");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", configuration.display());
    let package: serde_json::Value =
        serde_json::from_slice(&fs::read(manifest).expect("read plugin manifest"))
            .expect("parse plugin manifest");
    let configs: serde_json::Value =
        serde_json::from_slice(&fs::read(configuration).expect("read client configuration"))
            .expect("parse client configuration");
    let name = package["name"].as_str().expect("plugin name");
    let id = package["id"].as_str().expect("plugin id");
    let config = configs
        .get(id)
        .expect("client must have an explicit configuration");
    let list = |key: &str| {
        config[key]
            .as_array()
            .expect("string array")
            .iter()
            .map(|v| format!("{:?}", v.as_str().expect("configuration string")))
            .collect::<Vec<_>>()
            .join(",")
    };
    let mut generated = format!(
        "wit_bindgen::generate!({{path: {:?}, world: {:?}, generate_all}});\npub const NAME: &str = {:?};\npub const ID: &str = {:?};\npub(crate) const CONFIG: config::Configuration = config::Configuration {{languages: &[{}], program: {:?}, args: &[{}], markers: &[{}], outermost: {}, file_fallback: {}}};\n",
        wit.to_str().expect("UTF-8 WIT path"),
        // COMPAT(local-language-world): Existing clients import extension-ui.
        // Opted-in packages additionally use the resource interface; keep the
        // old ABI until those clients migrate. No removal version is scheduled.
        if env::var_os("CARGO_FEATURE_ENVIRONMENT").is_some() {
            "extension-workspace"
        } else {
            "extension-ui"
        },
        name,
        id,
        list("languages"),
        config["program"].as_str().expect("program"),
        list("args"),
        list("markers"),
        config["outermost"].as_bool().expect("outermost"),
        config["fileFallback"].as_bool().expect("fileFallback"),
    );
    generated.push_str(&format!(
        "pub const ENVIRONMENT_TOOLS: Option<&str> = {};\n",
        environment_tools(&crate_dir.join("../../.."), id)
    ));
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bindings.rs"),
        generated,
    )
    .unwrap();
}
