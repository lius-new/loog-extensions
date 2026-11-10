#[test]
fn selected_package_has_its_startup_configuration() {
    let config = crate::CONFIG;
    match ID {
        "loog.lsp.gopls" => {
            assert_eq!(config.languages, &["go"]);
            assert_eq!(config.program, "${extension}/gopls");
            assert!(!config.outermost);
        }
        "loog.lsp.rust-analyzer" => {
            assert_eq!(config.languages, &["rust"]);
            assert_eq!(config.program, "${extension}/rust-analyzer");
            assert_eq!(config.markers, &["Cargo.toml", "rust-project.json"]);
            assert!(config.outermost && config.file_fallback);
        }
        "loog.lsp.typescript-language-server" => {
            assert_eq!(config.languages, &["javascript", "typescript"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/typescript-language-server/lib/cli.mjs",
                    "--stdio"
                ]
            );
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.pyright" => {
            assert_eq!(config.languages, &["python"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/pyright/langserver.index.js",
                    "--stdio"
                ]
            );
            assert!(config.markers.contains(&"pyproject.toml"));
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.yaml-language-server" => {
            assert_eq!(config.languages, &["yaml"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/yaml-language-server/bin/yaml-language-server",
                    "--stdio"
                ]
            );
            assert!(config.markers.is_empty() && !config.file_fallback);
        }
        "loog.lsp.bash-language-server" => {
            assert_eq!(config.languages, &["bash"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/bash-language-server/out/cli.js",
                    "start"
                ]
            );
            assert_eq!(
                language_id("bash", Some("bash"), "script.sh"),
                "shellscript"
            );
            assert!(config.markers.is_empty() && !config.file_fallback);
        }
        "loog.lsp.clangd" => {
            assert_eq!(config.languages, &["c", "cpp", "objc"]);
            assert_eq!(language_id("objc", Some("objc"), "source.m"), "objective-c");
            assert_eq!(config.program, "${extension}/bin/clangd");
            assert!(config.args.is_empty());
            assert_eq!(
                config.markers,
                &["compile_commands.json", "compile_flags.txt", ".clangd"]
            );
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.lua-language-server" => {
            assert_eq!(config.languages, &["lua"]);
            assert_eq!(config.program, "${extension}/bin/lua-language-server");
            assert!(config.args.is_empty());
            assert_eq!(config.markers, &[".luarc.json", ".luarc.jsonc"]);
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.taplo" => {
            assert_eq!(config.languages, &["toml"]);
            assert_eq!(config.program, "${extension}/taplo");
            assert_eq!(config.args, &["lsp", "stdio"]);
            assert_eq!(config.markers, &[".taplo.toml", "taplo.toml"]);
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.css-language-server" => {
            assert_eq!(config.languages, &["css", "scss"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/vscode-langservers-extracted/bin/vscode-css-language-server",
                    "--stdio"
                ]
            );
            assert_eq!(config.markers, &["package.json"]);
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.html-language-server" => {
            assert_eq!(config.languages, &["html"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/vscode-langservers-extracted/bin/vscode-html-language-server",
                    "--stdio"
                ]
            );
            assert_eq!(config.markers, &["package.json"]);
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.biome" => {
            assert_eq!(
                config.languages,
                &["javascript", "typescript", "json", "css"]
            );
            assert_eq!(config.program, "${extension}/biome");
            assert_eq!(config.args, &["lsp-proxy"]);
            assert_eq!(config.markers, &["biome.json", "biome.jsonc"]);
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.marksman" => {
            assert_eq!(config.languages, &["markdown"]);
            assert_eq!(config.program, "${extension}/marksman");
            assert_eq!(config.args, &["server"]);
            assert_eq!(config.markers, &[".marksman.toml"]);
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.ruff" => {
            assert_eq!(config.languages, &["python"]);
            assert_eq!(config.program, "${extension}/ruff");
            assert_eq!(config.args, &["server"]);
            assert_eq!(
                config.markers,
                &["ruff.toml", ".ruff.toml", "pyproject.toml"]
            );
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.vscode-langservers-extracted" => {
            assert_eq!(config.languages, &["json"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/vscode-langservers-extracted/bin/vscode-json-language-server",
                    "--stdio"
                ]
            );
            assert_eq!(config.markers, &["package.json"]);
            assert!(!config.outermost && !config.file_fallback);
            assert_eq!(language_id("json", Some("json"), "settings.JSONC"), "jsonc");
            assert_eq!(language_id("json", Some("json"), "package.json"), "json");
        }
        "loog.lsp.eslint" => {
            assert_eq!(config.languages, &["javascript", "typescript"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/vscode-langservers-extracted/bin/vscode-eslint-language-server",
                    "--stdio"
                ]
            );
            assert_eq!(
                config.markers,
                &[
                    "eslint.config.js",
                    "eslint.config.mjs",
                    "eslint.config.cjs",
                    ".eslintrc.json",
                    ".eslintrc.js",
                    ".eslintrc.cjs",
                    ".eslintrc.yml",
                    ".eslintrc.yaml",
                    ".eslintrc",
                    "package.json"
                ]
            );
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.zls" => {
            assert_eq!(config.languages, &["zig"]);
            assert_eq!(config.program, "${extension}/zls");
            assert!(config.args.is_empty());
            assert_eq!(config.markers, &["build.zig", "build.zig.zon"]);
            assert!(!config.outermost && config.file_fallback);
        }
        "loog.lsp.phpantom" => {
            assert_eq!(config.languages, &["php"]);
            assert_eq!(config.program, "${extension}/phpantom_lsp");
            assert!(config.args.is_empty());
            assert_eq!(config.markers, &[".phpantom.toml", "composer.json"]);
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.svelte-language-server" => {
            assert_eq!(config.languages, &["svelte"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/svelte-language-server/bin/server.js",
                    "--stdio"
                ]
            );
            assert_eq!(
                config.markers,
                &[
                    "svelte.config.js",
                    "svelte.config.ts",
                    "svelte.config.mjs",
                    "svelte.config.cjs",
                    "package.json"
                ]
            );
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.dockerfile-language-server" => {
            assert_eq!(config.languages, &["dockerfile"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/dockerfile-language-server-nodejs/bin/docker-langserver",
                    "--stdio"
                ]
            );
            assert!(config.markers.is_empty());
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.neocmakelsp" => {
            assert_eq!(config.languages, &["cmake"]);
            assert_eq!(config.program, "${extension}/neocmakelsp");
            assert_eq!(config.args, &["stdio"]);
            assert_eq!(config.markers, &["CMakeLists.txt", "CMakePresets.json"]);
            assert!(config.outermost && !config.file_fallback);
        }
        "loog.lsp.roslyn" => {
            assert_eq!(config.languages, &["csharp"]);
            assert_eq!(config.program, "${extension}/roslyn");
            assert_eq!(
                config.args,
                &["--stdio", "--autoLoadProjects", "--telemetryLevel", "off"]
            );
            assert_eq!(
                config.markers,
                &[
                    "global.json",
                    "Directory.Build.props",
                    "Directory.Packages.props"
                ]
            );
            assert!(config.outermost && !config.file_fallback);
        }
        "loog.lsp.postgres-language-server" => {
            assert_eq!(config.languages, &["sql"]);
            assert_eq!(config.program, "${extension}/postgres-language-server");
            assert_eq!(config.args, &["lsp-proxy"]);
            assert_eq!(
                config.markers,
                &[
                    "postgres-language-server.jsonc",
                    "postgres-language-server.json"
                ]
            );
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.graphql-language-server" => {
            assert_eq!(config.languages, &["graphql"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(
                config.args,
                &[
                    "${extension}/node_modules/graphql-language-service-cli/bin/graphql.js",
                    "server",
                    "--method",
                    "stream"
                ]
            );
            assert_eq!(
                config.markers,
                &[
                    ".graphqlrc",
                    ".graphqlrc.json",
                    ".graphqlrc.yaml",
                    ".graphqlrc.yml",
                    ".graphqlrc.js",
                    ".graphqlrc.cjs",
                    ".graphqlrc.mjs",
                    ".graphqlrc.ts",
                    "graphql.config.json",
                    "graphql.config.yaml",
                    "graphql.config.yml",
                    "graphql.config.js",
                    "graphql.config.cjs",
                    "graphql.config.mjs",
                    "graphql.config.ts"
                ]
            );
            assert!(!config.outermost && !config.file_fallback);
        }
        "loog.lsp.jdtls" => {
            assert_eq!(config.languages, &["java"]);
            assert_eq!(config.program, "${runtime.node}/node");
            assert_eq!(config.args, &["${extension}/launch.mjs"]);
            assert_eq!(
                config.markers,
                &[
                    "pom.xml",
                    "build.gradle",
                    "build.gradle.kts",
                    "settings.gradle",
                    "settings.gradle.kts",
                    ".project"
                ]
            );
            assert!(config.outermost && !config.file_fallback);
        }
        "loog.lsp.buf" => {
            assert_eq!(config.languages, &["proto"]);
            assert_eq!(config.program, "${extension}/buf");
            assert_eq!(config.args, &["lsp", "serve"]);
            assert_eq!(config.markers, &["buf.yaml", "buf.work.yaml"]);
            assert!(config.outermost && !config.file_fallback);
        }
        _ => panic!("unconfigured package {ID}"),
    }
}

#[test]
fn rust_uses_outermost_project_and_file_fallback_while_ts_uses_nearest_marker() {
    let (mut c, _) = setup();
    let reference = DocumentRef {
        document: 7,
        lifecycle: 1,
        path: "C:/project/nested/src/lib.rs".into(),
        language: "rust".into(),
        variant: None,
        version: 0,
    };
    state(|s| {
        s.files.insert((1, "Cargo.toml".into()));
        s.files.insert((1, "nested/Cargo.toml".into()));
    });
    c.config = Configuration {
        languages: &["rust"],
        program: "${extension}/rust-analyzer",
        args: &[],
        markers: &["Cargo.toml", "rust-project.json"],
        outermost: true,
        file_fallback: true,
    };
    assert_eq!(c.root(&reference).unwrap().1, "");
    state(|s| s.files.clear());
    assert_eq!(c.root(&reference).unwrap().1, "nested/src");
    c.config = Configuration {
        languages: &["javascript", "typescript"],
        program: "${runtime.node}/node",
        args: &[],
        markers: &["tsconfig.json", "jsconfig.json", "package.json"],
        outermost: false,
        file_fallback: false,
    };
    state(|s| {
        s.files.insert((1, "package.json".into()));
        s.files.insert((1, "nested/tsconfig.json".into()));
    });
    assert_eq!(c.root(&reference).unwrap().1, "nested");
}

#[test]
fn jsx_and_tsx_are_synchronized_with_react_protocol_ids_and_matching_selectors() {
    let (mut c, p) = setup();
    c.config = Configuration {
        languages: &["javascript", "typescript"],
        program: "${runtime.node}/node",
        args: &[],
        markers: &["package.json"],
        outermost: false,
        file_fallback: false,
    };
    for (id, language, variant, protocol) in [
        (8, "typescript", "tsx", "typescriptreact"),
        (9, "javascript", "jsx", "javascriptreact"),
    ] {
        let reference = DocumentRef {
            document: id,
            lifecycle: 1,
            path: format!("C:/project/view.{variant}"),
            language: language.into(),
            variant: Some(variant.into()),
            version: 0,
        };
        state(|s| {
            s.documents
                .insert(id, (reference.clone(), "const view = <div/>;\n".into()));
        });
        c.message(Inbound::DocumentOpened(reference.clone()))
            .unwrap();
        assert_eq!(
            sent("textDocument/didOpen")["params"]["textDocument"]["languageId"],
            protocol
        );
        assert!(selector(&json!([{"language":protocol}]), &reference));
        assert!(!selector(&json!([{"language":"go"}]), &reference));
        assert_eq!(c.documents[&id].process, p);
    }
}
