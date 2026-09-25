use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    fs,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryFile {
    schema_version: u32,
    #[serde(default)]
    tools: Vec<ToolSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolSpec {
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    help: String,
    #[serde(default)]
    required: Vec<String>,
    #[serde(default)]
    providers: Vec<String>,
    resources: String,
}

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

fn valid_provider(provider: &str, registry: &RegistryFile) -> bool {
    registry
        .tools
        .iter()
        .any(|tool| tool.providers.iter().any(|known| known == provider))
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

fn validate(file: &CatalogFile, registry: &RegistryFile) -> Result<(), String> {
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
        if node.examples.is_empty() {
            return Err(format!("missing example: {}", node.path));
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
                registry,
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
            && !valid_provider(provider, registry)
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
            validate_option(option, &format!("{where_}.options"), registry)?;
        }
    }

    for (canonical, aliases) in &file.aliases {
        if canonical.trim().is_empty() || aliases.iter().any(|alias| alias.trim().is_empty()) {
            return Err("aliases cannot contain empty names".to_owned());
        }
    }
    for node in &file.nodes {
        for child in &node.children {
            if !node_paths.contains(child) {
                return Err(format!(
                    "node {} references missing child {child}",
                    node.path
                ));
            }
        }
    }
    Ok(())
}

fn validate_option(
    option: &OptionSpec,
    where_: &str,
    registry: &RegistryFile,
) -> Result<(), String> {
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
        && !valid_provider(provider, registry)
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

fn validate_registry(registry: &RegistryFile, catalog: &CatalogFile) -> Result<(), String> {
    if registry.schema_version != 1 {
        return Err(format!(
            "unsupported tool registry schema {}",
            registry.schema_version
        ));
    }
    let mut names = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let mut providers = BTreeSet::new();
    let paths = catalog
        .nodes
        .iter()
        .map(|node| node.path.as_str())
        .collect::<BTreeSet<_>>();
    for tool in &registry.tools {
        if tool.name.trim().is_empty() || !names.insert(tool.name.to_ascii_lowercase()) {
            return Err(format!("duplicate or empty tool name: {}", tool.name));
        }
        if !paths.contains(tool.name.as_str()) {
            return Err(format!("tool {} has no built-in root node", tool.name));
        }
        if tool.help.trim().is_empty()
            || !matches!(tool.resources.as_str(), "none" | "local" | "manual")
        {
            return Err(format!(
                "tool {} has invalid help/resource policy",
                tool.name
            ));
        }
        for alias in &tool.aliases {
            if alias.trim().is_empty() || !aliases.insert(alias.to_ascii_lowercase()) {
                return Err(format!("duplicate or empty tool alias: {alias}"));
            }
            let registered = catalog
                .aliases
                .get(&tool.name)
                .is_some_and(|values| values.iter().any(|value| value.eq_ignore_ascii_case(alias)));
            if !registered {
                return Err(format!(
                    "tool {} alias {alias} is missing from catalog",
                    tool.name
                ));
            }
        }
        for required in &tool.required {
            let path = format!("{} {required}", tool.name);
            if !paths.contains(path.as_str()) {
                return Err(format!(
                    "tool {} is missing required context {required}",
                    tool.name
                ));
            }
        }
        for provider in &tool.providers {
            if provider.trim().is_empty() || !providers.insert(provider.clone()) {
                return Err(format!("duplicate or empty provider: {provider}"));
            }
        }
    }
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
    out.push_str("// @generated by build.rs from specs/builtin/*.toml; do not edit.\n");
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

fn generate_registry(registry: &RegistryFile) -> String {
    let mut out = String::from("// @generated by build.rs from specs/tools.toml; do not edit.\n");
    out.push_str("pub static TOOLS: &[ToolDefinition] = &[\n");
    for tool in &registry.tools {
        let _ = writeln!(
            out,
            "    ToolDefinition {{ name: {}, aliases: {}, help: {}, required: {}, providers: {}, resources: {} }},",
            rust_str(&tool.name),
            rust_str_slice(&tool.aliases),
            rust_str(&tool.help),
            rust_str_slice(&tool.required),
            rust_str_slice(&tool.providers),
            rust_str(&tool.resources),
        );
    }
    out.push_str("];\n");
    out
}

#[cfg(windows)]
fn embed_legacy_json() {
    println!("cargo:rerun-if-changed=shell/legacy-json.cs");
    let windows =
        PathBuf::from(env::var_os("SystemRoot").expect("SystemRoot is required on Windows"));
    let shell = windows.join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let compiler = windows.join("Microsoft.NET/Framework64/v4.0.30319/csc.exe");
    let assembly = Command::new(shell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[System.Management.Automation.PSObject].Assembly.Location",
        ])
        .output()
        .expect("query Windows PowerShell 5.1 automation assembly");
    assert!(
        assembly.status.success(),
        "cannot locate Windows PowerShell 5.1 automation assembly"
    );
    let automation = PathBuf::from(
        String::from_utf8(assembly.stdout)
            .expect("assembly path is UTF-8")
            .trim(),
    );
    assert!(
        automation.is_file(),
        "missing Windows PowerShell automation assembly: {}",
        automation.display()
    );
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let dll = output.join("legacy-json.dll");
    let source = env::current_dir()
        .expect("read build directory")
        .join("shell")
        .join("legacy-json.cs");
    let compile = Command::new(compiler)
        .args([
            "/nologo".to_owned(),
            "/target:library".to_owned(),
            format!("/out:{}", dll.display()),
            "/reference:System.Web.Extensions.dll".to_owned(),
            "/reference:System.Core.dll".to_owned(),
            format!("/reference:{}", automation.display()),
            source.to_string_lossy().into_owned(),
        ])
        .output()
        .expect("compile PowerShell 5.1 JSON helper");
    assert!(
        compile.status.success(),
        "PowerShell 5.1 JSON helper compilation failed: {} {}",
        String::from_utf8_lossy(&compile.stdout),
        String::from_utf8_lossy(&compile.stderr)
    );
    let bytes = fs::read(dll).expect("read compiled PowerShell 5.1 JSON helper");
    assert!(
        !bytes.is_empty(),
        "compiled PowerShell 5.1 JSON helper is empty"
    );
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let value = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        encoded.push(TABLE[((value >> 18) & 63) as usize] as char);
        encoded.push(TABLE[((value >> 12) & 63) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            TABLE[((value >> 6) & 63) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            TABLE[(value & 63) as usize] as char
        } else {
            '='
        });
    }
    fs::write(output.join("legacy-json.base64"), encoded)
        .expect("write embedded JSON helper base64");
}

fn main() {
    #[cfg(windows)]
    embed_legacy_json();
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    let commit = env::var("GITHUB_SHA")
        .ok()
        .or_else(|| {
            Command::new("git")
                .args(["rev-parse", "HEAD"])
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|value| value.trim().to_owned())
        })
        .unwrap_or_else(|| "unknown".into());
    let build_time = env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        });
    println!("cargo:rustc-env=BLUEBERRY_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=BLUEBERRY_BUILD_TIME_UNIX={build_time}");
    println!("cargo:rerun-if-changed=specs/builtin");
    println!("cargo:rerun-if-changed=specs/tools.toml");
    let catalog_dir = PathBuf::from("specs/builtin");
    let mut sources = fs::read_dir(&catalog_dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", catalog_dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "toml")
        })
        .collect::<Vec<_>>();
    sources.sort();
    let mut file = CatalogFile {
        schema_version: SCHEMA_VERSION,
        name: String::new(),
        version: String::new(),
        aliases: BTreeMap::new(),
        option_sets: Vec::new(),
        nodes: Vec::new(),
    };
    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
        let text = fs::read_to_string(source)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", source.display()));
        let fragment: CatalogFile = toml::from_str(&text)
            .unwrap_or_else(|error| panic!("cannot parse {}: {error}", source.display()));
        if fragment.schema_version != SCHEMA_VERSION {
            panic!("invalid schema in {}", source.display());
        }
        if !fragment.name.is_empty() {
            file.name = fragment.name;
        }
        if !fragment.version.is_empty() {
            file.version = fragment.version;
        }
        for (canonical, aliases) in fragment.aliases {
            if file.aliases.insert(canonical.clone(), aliases).is_some() {
                panic!("duplicate alias root {canonical} in {}", source.display());
            }
        }
        file.option_sets.extend(fragment.option_sets);
        file.nodes.extend(fragment.nodes);
    }
    if sources.is_empty() || file.name.is_empty() || file.version.is_empty() {
        panic!("specs/builtin must contain catalog metadata and tool fragments");
    }
    for node in &mut file.nodes {
        for child in &mut node.children {
            if !child.contains(' ') {
                *child = format!("{} {child}", node.path);
            }
        }
    }
    let registry_source = PathBuf::from("specs/tools.toml");
    let registry_text = fs::read_to_string(&registry_source).unwrap_or_else(|error| {
        panic!("cannot read {}: {error}", registry_source.display());
    });
    let registry: RegistryFile = toml::from_str(&registry_text).unwrap_or_else(|error| {
        panic!("cannot parse {}: {error}", registry_source.display());
    });
    if let Err(error) = validate_registry(&registry, &file) {
        panic!("invalid {}: {error}", registry_source.display());
    }
    if let Err(error) = validate(&file, &registry) {
        panic!("invalid built-in catalog: {error}");
    }
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    fs::write(out_dir.join("builtin_specs.rs"), generate(&file))
        .expect("write generated builtin_specs.rs");
    fs::write(
        out_dir.join("tool_registry.rs"),
        generate_registry(&registry),
    )
    .expect("write generated tool_registry.rs");
}
