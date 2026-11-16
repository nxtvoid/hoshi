//! Languages offered for a code block, by the id written after its opening fence.

use crate::theme::Colors;
use gpui_kit::Hsla;

pub struct CodeLanguage {
    pub id: &'static str,
    pub label: &'static str,
    pub aliases: &'static [&'static str],
}

const fn language(
    id: &'static str,
    label: &'static str,
    aliases: &'static [&'static str],
) -> CodeLanguage {
    CodeLanguage { id, label, aliases }
}

pub const CODE_LANGUAGES: &[CodeLanguage] = &[
    language("typescript", "TypeScript", &["ts"]),
    language("tsx", "TSX", &[]),
    language("javascript", "JavaScript", &["js"]),
    language("jsx", "JSX", &[]),
    language("json", "JSON", &[]),
    language("rust", "Rust", &["rs"]),
    language("python", "Python", &["py"]),
    language("go", "Go", &[]),
    language("java", "Java", &[]),
    language("kotlin", "Kotlin", &["kt"]),
    language("swift", "Swift", &[]),
    language("c", "C", &[]),
    language("cpp", "C++", &["c++"]),
    language("csharp", "C#", &["cs", "c#"]),
    language("ruby", "Ruby", &["rb"]),
    language("php", "PHP", &[]),
    language("html", "HTML", &[]),
    language("css", "CSS", &[]),
    language("scss", "SCSS", &[]),
    language("sql", "SQL", &[]),
    language("bash", "Bash", &["sh", "shell", "zsh"]),
    language("powershell", "PowerShell", &["ps1", "pwsh"]),
    language("yaml", "YAML", &["yml"]),
    language("toml", "TOML", &[]),
    language("markdown", "Markdown", &["md"]),
    language("diff", "Diff", &[]),
    language("dockerfile", "Dockerfile", &["docker"]),
    language("xml", "XML", &[]),
    language("lua", "Lua", &[]),
];

/// How a block's language is shown: its name, or the id as written.
pub fn code_language_label(id: &str) -> String {
    let needle = id.trim().to_lowercase();
    CODE_LANGUAGES
        .iter()
        .find(|language| language.id == needle || language.aliases.contains(&needle.as_str()))
        .map_or_else(|| id.to_owned(), |language| language.label.to_owned())
}

/// The languages whose name, id or alias contains `query`, ignoring case.
pub fn search_code_languages(query: &str) -> Vec<&'static CodeLanguage> {
    let needle = query.trim().to_lowercase();
    CODE_LANGUAGES
        .iter()
        .filter(|language| {
            needle.is_empty()
                || language.label.to_lowercase().contains(&needle)
                || language.id.contains(&needle)
                || language.aliases.iter().any(|alias| alias.contains(&needle))
        })
        .collect()
}

/// The language an id or alias names, as its id; the id as written when it names none.
pub fn code_language_id(id: &str) -> String {
    let needle = id.trim().to_lowercase();
    CODE_LANGUAGES
        .iter()
        .find(|language| language.id == needle || language.aliases.contains(&needle.as_str()))
        .map_or_else(|| id.to_owned(), |language| language.id.to_owned())
}

/// The file-type icon a language shows, as its name in `assets/file-icons`, and its color.
pub fn code_language_icon(id: &str) -> (&'static str, fn(&Colors) -> Hsla) {
    match code_language_id(id).as_str() {
        "" => ("text", |colors| colors.file_gray),
        "typescript" => ("typescript", |colors| colors.file_blue),
        "tsx" | "jsx" => ("react", |colors| colors.file_cyan),
        "javascript" => ("javascript", |colors| colors.file_yellow),
        "json" => ("json", |colors| colors.file_orange),
        "rust" => ("rust", |colors| colors.file_orange),
        "python" => ("python", |colors| colors.file_blue),
        "go" => ("go", |colors| colors.file_cyan),
        "swift" => ("swift", |colors| colors.file_orange),
        "c" => ("c", |colors| colors.file_blue),
        "cpp" => ("cpp", |colors| colors.file_blue),
        "ruby" => ("ruby", |colors| colors.file_red),
        "html" => ("html", |colors| colors.file_orange),
        "css" => ("css", |colors| colors.file_purple),
        "scss" => ("sass", |colors| colors.file_pink),
        "sql" => ("database", |colors| colors.file_magenta),
        "bash" => ("bash", |colors| colors.file_green),
        "yaml" => ("yml", |colors| colors.file_red),
        "markdown" => ("markdown", |colors| colors.file_green),
        "dockerfile" => ("docker", |colors| colors.file_blue),
        _ => ("default", |colors| colors.file_gray),
    }
}

/// The file-type icon of a path, by its extension, or by its whole name when it has none,
/// as `Dockerfile`.
pub fn file_path_icon(path: &str) -> (&'static str, fn(&Colors) -> Hsla) {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    code_language_icon(
        name.rsplit_once('.')
            .map_or(name, |(_, extension)| extension),
    )
}

#[cfg(test)]
mod tests {
    use super::{code_language_icon, code_language_id, file_path_icon, search_code_languages};

    #[test]
    fn languages_are_found_by_name_id_or_alias() {
        let ids = |query| {
            search_code_languages(query)
                .into_iter()
                .map(|language| language.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("type"), ["typescript"]);
        assert_eq!(ids("rs"), ["rust", "powershell"]);
        assert_eq!(ids("C#"), ["csharp"]);
        assert_eq!(ids("").len(), 29);
        assert_eq!(code_language_id("TS"), "typescript");
        assert_eq!(code_language_id("weird"), "weird");
        assert_eq!(code_language_icon("tsx").0, "react");
        assert_eq!(code_language_icon("weird").0, "default");
    }

    #[test]
    fn a_path_shows_the_icon_of_its_extension_or_its_name() {
        assert_eq!(file_path_icon("src/app/main.rs").0, "rust");
        assert_eq!(file_path_icon(r"web\Button.tsx").0, "react");
        assert_eq!(file_path_icon("deploy/Dockerfile").0, "docker");
        assert_eq!(file_path_icon("notes.v2.md").0, "markdown");
        assert_eq!(file_path_icon("Makefile").0, "default");
    }
}
