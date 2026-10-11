#[must_use]
pub fn language_for_path(file_path: &str) -> Option<&'static str> {
    tree_sitter_language_pack::detect_language(file_path)
}

pub(crate) fn comment_query(name: &str) -> &'static str {
    match name {
        "rust" | "java" | "elm" | "groovy" => "(line_comment) @comment\n(block_comment) @comment",
        "kotlin" => "(line_comment) @comment\n(multiline_comment) @comment",
        _ => "(comment) @comment",
    }
}

pub(crate) fn docstring_prefix(name: &str) -> Option<&'static str> {
    matches!(name, "javascript" | "typescript" | "tsx" | "java").then_some("/**")
}

pub(crate) fn docstring_query(name: &str) -> Option<&'static str> {
    match name {
        "python" => Some(
            r"
		(module . (string) @docstring)
		(class_definition body: (block . (string) @docstring))
		(function_definition body: (block . (string) @docstring))
	",
        ),
        "javascript" | "typescript" | "tsx" => Some(
            r"
		(comment) @jsdoc
	",
        ),
        "java" => Some(
            r"
		(block_comment) @javadoc
	",
        ),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::language_for_path;

    #[test]
    fn language_for_path_given_known_extension_returns_name() {
        // then
        assert_eq!(language_for_path("main.py"), Some("python"));
    }

    #[test]
    fn language_for_path_given_unknown_extension_returns_none() {
        // then
        assert_eq!(language_for_path("main.xyz"), None);
    }

    #[test]
    fn language_for_path_given_known_extensionless_path_still_resolves() {
        // then
        assert_eq!(language_for_path("Dockerfile"), Some("dockerfile"));
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
            "main.ex",
            "Main.elm",
            "build.groovy",
            "config.hcl",
            "main.ml",
            "main.proto",
            "App.svelte",
            "config.cue",
            "Dockerfile",
        ];

        // when
        let unloaded: Vec<&str> = paths
            .iter()
            .copied()
            .filter(|path| {
                language_for_path(path).is_none_or(|name| {
                    !tree_sitter_language_pack::has_language(name)
                        || tree_sitter_language_pack::get_language(name).is_err()
                })
            })
            .collect();

        // then
        assert!(tree_sitter_language_pack::language_count() > 0);
        assert!(unloaded.is_empty(), "unloaded paths: {unloaded:?}");
    }
}
