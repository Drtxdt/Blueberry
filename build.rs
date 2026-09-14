use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    fs,
    path::PathBuf,
};

const SCHEMA_VERSION: u32 = 1;

const ALLOWED_PROVIDERS: &[&str] = &[
    "git",
    "git.refs",
    "git.branches",
    "git.remotes",
    "git.tags",
    "git.worktrees",
    "git.status",
    "git.paths",
    "cargo.packages",
    "cargo.features",
    "cargo.bins",
    "cargo.examples",
    "cargo.tests",
    "cargo.benches",
    "npm.scripts",
    "npm.workspaces",
    "npm.dependencies",
    "pnpm.scripts",
    "pnpm.workspaces",
    "pnpm.dependencies",
    "python.scripts", "python.dependencies", "python.environments",
    "conda.environments", "conda.packages",
    "poetry.scripts", "poetry.dependencies", "poetry.environments",
    "yarn.scripts", "yarn.workspaces", "yarn.dependencies",
    "bun.scripts", "bun.workspaces", "bun.dependencies",
    "rustup.toolchains", "rustup.targets", "rustup.components",
    "go.packages", "go.files", "go.workspaces",
    "dotnet.projects", "dotnet.frameworks", "dotnet.references", "dotnet.tools",
    "cmake.presets", "cmake.build_presets", "cmake.test_presets", "cmake.targets",
    "docker.services", "docker.profiles", "docker.contexts",
    "kubectl.contexts", "kubectl.namespaces", "kubectl.resources",
    "helm.charts", "helm.repositories", "helm.releases",
    "ssh.hosts",
    "powershell.env",
    "powershell.paths",
    "powershell.redirects",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    schema_version: u32,
    #[serde(default)]
    name: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    aliases: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    option_sets: Vec<OptionSetSpec>,
    #[serde(default)]
    nodes: Vec<NodeSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionSetSpec {
    id: String,
    #[serde(default)]
    options: Vec<OptionSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionSpec {
    name: String,
    #[serde(default)]
    names: Vec<String>,
    description: String,
    #[serde(default)]
    detail: String,
    value_kind: String,
    #[serde(default)]
    values: Vec<ValueSpec>,
    #[serde(default)]
    optional: bool,
    #[serde(default)]
    repeatable: bool,
    #[serde(default)]
    conflicts: Vec<String>,
    #[serde(default)]
    requires: Vec<String>,
    #[serde(default)]
    positional: bool,
    provider: Option<String>,
    value_delimiter: Option<String>,
    #[serde(default = "default_true")]
    append_space: bool,
    #[serde(default)]
    short_cluster: bool,
    #[serde(default)]
    value_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ValueSpec {
    name: String,
    description: String,
    #[serde(default)]
    detail: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeSpec {
    path: String,
    #[serde(default)]
    name: String,
    description: String,
    #[serde(default)]
    detail: String,
    positional: String,
    #[serde(default)]
    children: Vec<String>,
    #[serde(default)]
    option_sets: Vec<String>,
    #[serde(default)]
    options: Vec<OptionSpec>,
    #[serde(default)]
    examples: Vec<String>,
    #[serde(default)]
    keywords: Vec<String>,
    provider: Option<String>,
    value_delimiter: Option<String>,
}

fn default_true() -> bool {
    true
}

fn has_chinese(value: &str) -> bool {
    value
        .chars()
        .any(|character| ('\u{3400}'..='\u{9fff}').contains(&character))
}

fn valid_provider(provider: &str) -> bool {
    ALLOWED_PROVIDERS.contains(&provider)
}

fn parse_delimiter(value: &Option<String>, where_: &str) -> Result<Option<char>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    let mut chars = value.chars();
    let Some(character) = chars.next() else {
        return Err(format!("{where_}: value_delimiter cannot be empty"));
    };
    if chars.next().is_some() {
        return Err(format!(
            "{where_}: value_delimiter must contain exactly one character"
        ));
    }
    Ok(Some(character))
}

fn validate(file: &CatalogFile) -> Result<(), String> {
    if file.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "schema_version {} is unsupported; expected {}",
            file.schema_version, SCHEMA_VERSION
        ));
    }
    for set in &file.option_sets {
        if !file.nodes.iter().any(|n| n.option_sets.contains(&set.id)) {
            return Err(format!("unreferenced option set: {}", set.id));
        }
    }
    for node in &file.nodes {
        if node.detail.contains("按 F1 查看") {
            return Err(format!("placeholder detail: {}", node.path));
        }
        for example in &node.examples {
            if example.trim().is_empty() || example.contains('\n') || example.contains('\x1b') {
                return Err(format!("invalid example: {}", node.path));
            }
        }
    }
    let mut option_ids = BTreeSet::new();
    for set in &file.option_sets {
        if set.id.trim().is_empty() {
            return Err("option set id cannot be empty".to_owned());
        }
        if !option_ids.insert(&set.id) {
            return Err(format!("duplicate option set: {}", set.id));
        }
        let mut names = BTreeSet::new();
        for (index, option) in set.options.iter().enumerate() {
            validate_option(
                option,
                &format!("option_sets[{}].options[{}]", set.id, index),
            )?;
            if !names.insert(&option.name) {
                return Err(format!(
                    "duplicate option {} in set {}",
                    option.name, set.id
                ));
            }
        }
    }

    let mut node_paths = BTreeSet::new();
    for (index, node) in file.nodes.iter().enumerate() {
        let where_ = format!("nodes[{index}] ({})", node.path);
        if node.path.trim().is_empty() || node.path.split_whitespace().next().is_none() {
            return Err(format!("{where_}: path cannot be empty"));
        }
        if !node_paths.insert(&node.path) {
            return Err(format!("duplicate node path: {}", node.path));
        }
        if node.description.trim().is_empty() || !has_chinese(&node.description) {
            return Err(format!(
                "{where_}: description must be non-empty Chinese text"
            ));
        }
        if node.detail.trim().is_empty() || !has_chinese(&node.detail) {
            return Err(format!("{where_}: detail must be non-empty Chinese text"));
        }
        if !matches!(
            node.positional.as_str(),
            "none" | "value" | "path" | "directory"
        ) {
            return Err(format!(
                "{where_}: positional must be none, value, path, or directory"
            ));
        }
        if let Some(provider) = &node.provider
            && !valid_provider(provider)
        {
            return Err(format!("{where_}: unknown provider {provider}"));
        }
        parse_delimiter(&node.value_delimiter, &where_)?;
        for option_set in &node.option_sets {
            if !option_ids.contains(option_set) {
                return Err(format!("{where_}: unknown option set {option_set}"));
            }
        }
        for option in &node.options {
            validate_option(option, &format!("{where_}.options"))?;
        }
    }

    for (canonical, aliases) in &file.aliases {
        if canonical.trim().is_empty() || aliases.iter().any(|alias| alias.trim().is_empty()) {
            return Err("aliases cannot contain empty names".to_owned());
        }
    }
    Ok(())
}

fn validate_option(option: &OptionSpec, where_: &str) -> Result<(), String> {
    if !option.name.starts_with('-')
        || option.description == "参数选项"
        || option.detail.contains("按 F1 查看")
    {
        return Err(format!(
            "{where_}: invalid option or placeholder documentation"
        ));
    }
    if option.name.trim().is_empty() || option.name.chars().any(char::is_whitespace) {
        return Err(format!(
            "{where_}: option name must be non-empty and contain no spaces"
        ));
    }
    if option.description.trim().is_empty() || !has_chinese(&option.description) {
        return Err(format!(
            "{where_} {}: description must be non-empty Chinese text",
            option.name
        ));
    }
    if option.detail.trim().is_empty() || !has_chinese(&option.detail) {
        return Err(format!(
            "{where_} {}: detail must be non-empty Chinese text",
            option.name
        ));
    }
    if !matches!(
        option.value_kind.as_str(),
        "none" | "text" | "path" | "directory"
    ) {
        return Err(format!(
            "{where_} {}: value_kind must be none, text, path, or directory",
            option.name
        ));
    }
    if option.value_kind == "none" && !option.values.is_empty() {
        return Err(format!(
            "{where_} {}: flag options cannot have values",
            option.name
        ));
    }
    if option.value_kind != "none" && option.optional && option.values.is_empty() {
        // Free-form optional values are valid, for example --pretty[=<format>].
        // Keep this explicit branch so the rule is visible to future schema checks.
    }
    for value in &option.values {
        if value.name.trim().is_empty() || value.description.trim().is_empty() {
            return Err(format!(
                "{where_} {}: value names and descriptions are required",
                option.name
            ));
        }
        if !has_chinese(&value.description) {
            return Err(format!(
                "{where_} {}: value description must contain Chinese",
                option.name
            ));
        }
        if !value.detail.is_empty() && !has_chinese(&value.detail) {
            return Err(format!(
                "{where_} {}: value detail must contain Chinese",
                option.name
            ));
        }
    }
    if let Some(provider) = &option.provider
        && !valid_provider(provider)
    {
        return Err(format!(
            "{where_} {}: unknown provider {provider}",
            option.name
        ));
    }
    parse_delimiter(
        &option.value_delimiter,
        &format!("{where_} {}", option.name),
    )?;
    Ok(())
}

fn rust_str(value: &str) -> String {
    format!("{value:?}")
}

fn rust_str_slice(values: &[String]) -> String {
    if values.is_empty() {
        "&[]".to_owned()
    } else {
        format!(
            "&[{}]",
            values
                .iter()
                .map(|value| rust_str(value))
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

fn rust_opt_str(value: &Option<String>) -> String {
    value
        .as_deref()
        .map(|value| format!("Some({})", rust_str(value)))
        .unwrap_or_else(|| "None".to_owned())
}

fn rust_opt_char(value: &Option<char>) -> String {
    value
        .map(|value| format!("Some({value:?})"))
        .unwrap_or_else(|| "None".to_owned())
}

fn expand_names(option: &OptionSpec) -> Vec<String> {
    let mut names = Vec::with_capacity(option.names.len() + 1);
    names.push(option.name.clone());
    for name in &option.names {
        if !names.contains(name) {
            names.push(name.clone());
        }
    }
    names
}

fn option_static(out: &mut String, option: &OptionSpec) {
    let delimiter = parse_delimiter(&option.value_delimiter, "generated option")
        .expect("catalog was validated");
    let mut values = String::from("&[");
    for (index, value) in option.values.iter().enumerate() {
        if index != 0 {
            values.push_str(", ");
        }
        let detail = if value.detail.is_empty() {
            value.description.clone()
        } else {
            value.detail.clone()
        };
        let _ = write!(
            values,
            "StaticValue {{ name: {}, description: {}, detail: {} }}",
            rust_str(&value.name),
            rust_str(&value.description),
            rust_str(&detail)
        );
    }
    values.push(']');
    let _ = writeln!(
        out,
        "        StaticOption {{ names: {}, description: {}, detail: {}, value_kind: {}, values: {}, optional_value: {}, repeatable: {}, conflicts: {}, requires: {}, positional: {}, provider: {}, value_delimiter: {}, append_space: {}, short_cluster: {}, value_name: {} }},",
        rust_str_slice(&expand_names(option)),
        rust_str(&option.description),
        rust_str(&option.detail),
        rust_str(&option.value_kind),
        values,
        option.optional,
        option.repeatable,
        rust_str_slice(&option.conflicts),
        rust_str_slice(&option.requires),
        option.positional,
        rust_opt_str(&option.provider),
        rust_opt_char(&delimiter),
        option.append_space,
        option.short_cluster,
        rust_str(&option.value_name),
    );
}

fn generate(file: &CatalogFile) -> String {
    let mut out = String::new();
    out.push_str("// @generated by build.rs from specs/builtin.toml; do not edit.\n");
    let _ = writeln!(
        out,
        "pub(crate) static BUILTIN_CATALOG_NAME: &str = {};",
        rust_str(&file.name)
    );
    let _ = writeln!(
        out,
        "pub(crate) static BUILTIN_CATALOG_VERSION: &str = {};\n",
        rust_str(&file.version)
    );
    out.push_str("pub(crate) static BUILTIN_OPTION_SETS: &[StaticOptionSet] = &[\n");
    for set in &file.option_sets {
        let _ = writeln!(
            out,
            "    StaticOptionSet {{ id: {}, options: &[",
            rust_str(&set.id)
        );
        for option in &set.options {
            option_static(&mut out, option);
        }
        out.push_str("    ] },\n");
    }
    out.push_str("];\n\n");

    out.push_str("pub(crate) static BUILTIN_NODES: &[StaticNode] = &[\n");
    for node in &file.nodes {
        let positional = rust_str(&node.positional);
        let _ = writeln!(
            out,
            "    StaticNode {{ path: {}, name: {}, description: {}, detail: {}, positional: {}, children: {}, option_sets: {}, options: &[], examples: {}, keywords: {}, provider: {}, value_delimiter: {} }},",
            rust_str(&node.path),
            rust_str(&if node.name.is_empty() {
                node.path
                    .rsplit_once(' ')
                    .map(|(_, name)| name.to_owned())
                    .unwrap_or_else(|| node.path.clone())
            } else {
                node.name.clone()
            }),
            rust_str(&node.description),
            rust_str(&node.detail),
            positional,
            rust_str_slice(&node.children),
            rust_str_slice(&node.option_sets),
            rust_str_slice(&node.examples),
            rust_str_slice(&node.keywords),
            rust_opt_str(&node.provider),
            rust_opt_char(
                &parse_delimiter(&node.value_delimiter, "generated node")
                    .expect("catalog was validated")
            ),
        );
    }
    out.push_str("];\n\n");

    out.push_str("pub(crate) static BUILTIN_ALIASES: &[StaticAlias] = &[\n");
    for (canonical, aliases) in &file.aliases {
        let _ = writeln!(
            out,
            "    StaticAlias {{ canonical: {}, aliases: {} }},",
            rust_str(canonical),
            rust_str_slice(aliases)
        );
    }
    out.push_str("];\n\n");

    out.push_str("pub(crate) static BUILTIN_ROOT_DESCRIPTIONS: &[(&str, &str)] = &[\n");
    for node in &file.nodes {
        if !node.path.contains(' ') {
            let _ = writeln!(
                out,
                "    ({}, {}),",
                rust_str(&node.path),
                rust_str(&node.description)
            );
        }
    }
    out.push_str("];\n\n");

    out.push_str("pub(crate) static BUILTIN_DESCRIPTION_INVENTORY: &[&str] = &[\n");
    for node in &file.nodes {
        let _ = writeln!(out, "    {},", rust_str(&node.description));
        let _ = writeln!(out, "    {},", rust_str(&node.detail));
    }
    for set in &file.option_sets {
        for option in &set.options {
            let _ = writeln!(out, "    {},", rust_str(&option.description));
            let _ = writeln!(out, "    {},", rust_str(&option.detail));
            for value in &option.values {
                let _ = writeln!(out, "    {},", rust_str(&value.description));
                if !value.detail.is_empty() {
                    let _ = writeln!(out, "    {},", rust_str(&value.detail));
                }
            }
        }
    }
    out.push_str("];\n");
    out
}

fn main() {
    println!("cargo:rerun-if-changed=specs/builtin.toml");
    let source = PathBuf::from("specs/builtin.toml");
    let text = fs::read_to_string(&source).unwrap_or_else(|error| {
        panic!("cannot read {}: {error}", source.display());
    });
    let file: CatalogFile = toml::from_str(&text).unwrap_or_else(|error| {
        panic!("cannot parse {}: {error}", source.display());
    });
    if let Err(error) = validate(&file) {
        panic!("invalid {}: {error}", source.display());
    }
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out_dir.join("builtin_specs.rs"), generate(&file))
        .expect("write generated builtin_specs.rs");
}
