#[test]
fn selected_package_keeps_its_original_startup_configuration() {
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
