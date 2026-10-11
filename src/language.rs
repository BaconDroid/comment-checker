#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LanguageSpec {
    pub name: &'static str,
    pub docstring_query: Option<&'static str>,
}

#[must_use]
pub fn language_for_path(file_path: &str) -> Option<LanguageSpec> {
    let name = tree_sitter_language_pack::detect_language(file_path)?;
    Some(LanguageSpec {
        name,
        docstring_query: docstring_query(name),
    })
}

fn docstring_query(name: &str) -> Option<&'static str> {
    match name {
        "python" => Some(
            r"
		(module . (string) @docstring)
		(class_definition body: (block . (string) @docstring))
		(function_definition body: (block . (string) @docstring))
	",
        ),
        "javascript" | "typescript" | "tsx" => Some(
            r#"
		(comment) @jsdoc
		(#match? @jsdoc "^/\*\*")
	"#,
        ),
        "java" => Some(
            r#"
		(block_comment) @javadoc
		(#match? @javadoc "^/\*\*")
	"#,
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::language_for_path;

    #[test]
    fn language_for_path_given_known_extension_returns_spec() {
        // given
        let spec = language_for_path("main.py");

        // then
        assert_eq!(spec.map(|spec| spec.name), Some("python"));
    }

    #[test]
    fn language_for_path_given_unknown_extension_returns_none() {
        // then
        assert!(language_for_path("main.xyz").is_none());
    }

    #[test]
    fn language_for_path_given_known_extensionless_path_still_resolves() {
        // then
        assert_eq!(
            language_for_path("Dockerfile").map(|spec| spec.name),
            Some("dockerfile")
        );
    }

    #[test]
    #[cfg_attr(
        miri,
        ignore = "tree-sitter language pack calls generated C parser FFI"
    )]
    fn language_for_path_given_supported_paths_loads_tree_sitter_languages() {
        // given
        let paths = [
            "main.py",
            "main.go",
            "main.ts",
            "main.tsx",
            "main.js",
            "Main.java",
            "main.c",
            "main.cpp",
            "main.rs",
            "main.rb",
            "script.sh",
            "Main.kt",
            "main.swift",
            "main.cs",
            "main.php",
            "main.lua",
            "main.sql",
            "index.html",
            "style.css",
            "config.yaml",
            "Cargo.toml",
            "Main.scala",
        ];

        // when
        let unloaded: Vec<&str> = paths
            .iter()
            .copied()
            .filter(|path| {
                language_for_path(path)
                    .is_none_or(|spec| tree_sitter_language_pack::get_language(spec.name).is_err())
            })
            .collect();

        // then
        assert!(unloaded.is_empty(), "unloaded paths: {unloaded:?}");
    }
}
