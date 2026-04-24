use std::{
    collections::{HashMap, HashSet},
    env, fs,
};

use anyhow::{Context, Result, bail};
use quote::ToTokens;
use syn::{Attribute, Expr, Fields, Item, ItemEnum, ItemStruct, Lit, Meta, Variant, parse_file};
use toml::Value;

const SECTION_ORDER: &[SectionSpec] = &[
    SectionSpec::new("[hotkey]", SectionKind::Table, "HotKeyConfig", &["hotkey"]),
    SectionSpec::new("[window]", SectionKind::Table, "WindowConfig", &["window"]),
    SectionSpec::new(
        "[providers]",
        SectionKind::Table,
        "ProvidersConfig",
        &["providers"],
    ),
    SectionSpec::new(
        "[providers.windows]",
        SectionKind::Table,
        "WindowsProviderConfig",
        &["providers", "windows"],
    ),
    SectionSpec::new(
        "[providers.apps]",
        SectionKind::Table,
        "AppsProviderConfig",
        &["providers", "apps"],
    ),
    SectionSpec::new(
        "[ranking]",
        SectionKind::Table,
        "RankingConfig",
        &["ranking"],
    ),
    SectionSpec::new(
        "[ranking.provider_score_boosts]",
        SectionKind::ProviderBoosts,
        "RankingConfig",
        &["ranking", "provider_score_boosts"],
    ),
    SectionSpec::new(
        "[[ranking.score_rules]]",
        SectionKind::RuleArray,
        "RankingScoreRule",
        &["ranking", "score_rules"],
    ),
    SectionSpec::new("[timing]", SectionKind::Table, "TimingConfig", &["timing"]),
    SectionSpec::new(
        "[plugins]",
        SectionKind::Table,
        "PluginsConfig",
        &["plugins"],
    ),
    SectionSpec::new(
        "[plugin.<id>]",
        SectionKind::PluginTable,
        "Config",
        &["plugin"],
    ),
    SectionSpec::new(
        "[plugin.<id>.commands]",
        SectionKind::PluginCommands,
        "Config",
        &["plugin"],
    ),
    SectionSpec::new("[ui]", SectionKind::Table, "UiConfig", &["ui"]),
    SectionSpec::new(
        "[ui.canvas]",
        SectionKind::Table,
        "UiCanvasConfig",
        &["ui", "canvas"],
    ),
    SectionSpec::new(
        "[ui.entries]",
        SectionKind::Table,
        "UiEntriesConfig",
        &["ui", "entries"],
    ),
    SectionSpec::new(
        "[ui.shortcuts]",
        SectionKind::Table,
        "UiShortcutsConfig",
        &["ui", "shortcuts"],
    ),
    SectionSpec::new(
        "[ui.font_sizes]",
        SectionKind::Table,
        "UiFontSizesConfig",
        &["ui", "font_sizes"],
    ),
    SectionSpec::new(
        "[ui.layout]",
        SectionKind::Table,
        "UiLayoutConfig",
        &["ui", "layout"],
    ),
    SectionSpec::new(
        "[ui.colorschemes.<name>]",
        SectionKind::Colorscheme,
        "UiColorschemeConfig",
        &["ui", "colorschemes"],
    ),
];

fn main() -> Result<()> {
    let root = env::current_dir().context("failed to resolve current working directory")?;
    let config_rs = root.join("src/config.rs");
    let output = root.join("CONFIGURATION.md");

    let source = fs::read_to_string(&config_rs)
        .with_context(|| format!("failed to read {}", config_rs.display()))?;
    let file = parse_file(&source).context("failed to parse src/config.rs with syn")?;

    let docs = parse_config_items(&file)?;
    let default_config = extract_default_config(&file)?;
    let defaults = default_config
        .parse::<Value>()
        .context("failed to parse DEFAULT_CONFIG as TOML")?;
    let provider_names = extract_string_array_const(&file, "KNOWN_PROVIDER_NAMES")?;
    let builtin_schemes = extract_string_array_const(&file, "BUILTIN_COLORSCHEME_NAMES")?;

    validate_source_schema(&docs, &defaults, &provider_names, &builtin_schemes)?;

    let rendered = render_document(&docs, &defaults, &provider_names, &builtin_schemes)?;

    if env::args().nth(1).as_deref() == Some("--check") {
        let current = fs::read_to_string(&output).unwrap_or_default();
        if current != rendered {
            bail!("CONFIGURATION.md is out of date; run scripts/generate-config-docs.sh");
        }
        return Ok(());
    }

    fs::write(&output, rendered)
        .with_context(|| format!("failed to write {}", output.display()))?;
    Ok(())
}

#[derive(Clone, Copy)]
struct SectionSpec {
    path: &'static str,
    kind: SectionKind,
    rust_type: &'static str,
    default_path: &'static [&'static str],
}

impl SectionSpec {
    const fn new(
        path: &'static str,
        kind: SectionKind,
        rust_type: &'static str,
        default_path: &'static [&'static str],
    ) -> Self {
        Self {
            path,
            kind,
            rust_type,
            default_path,
        }
    }
}

#[derive(Clone, Copy)]
enum SectionKind {
    Table,
    ProviderBoosts,
    RuleArray,
    PluginTable,
    PluginCommands,
    Colorscheme,
}

#[derive(Clone)]
struct StructDef {
    doc: String,
    fields: Vec<FieldDef>,
}

#[derive(Clone)]
struct FieldDef {
    rust_name: String,
    toml_name: String,
    rust_type: String,
    doc: String,
}

#[derive(Clone)]
struct EnumDef {
    variants: Vec<String>,
}

#[derive(Clone)]
enum DocItem {
    Struct(StructDef),
    Enum(EnumDef),
}

type DocMap = HashMap<String, DocItem>;

fn parse_config_items(file: &syn::File) -> Result<DocMap> {
    let mut items = HashMap::new();

    for item in &file.items {
        match item {
            Item::Struct(item_struct) if matches!(item_struct.fields, Fields::Named(_)) => {
                items.insert(
                    item_struct.ident.to_string(),
                    DocItem::Struct(parse_struct(item_struct)?),
                );
            }
            Item::Enum(item_enum) => {
                items.insert(
                    item_enum.ident.to_string(),
                    DocItem::Enum(parse_enum(item_enum)),
                );
            }
            _ => {}
        }
    }

    Ok(items)
}

fn parse_struct(item: &ItemStruct) -> Result<StructDef> {
    let Fields::Named(named) = &item.fields else {
        bail!("{} is not a named struct", item.ident);
    };

    let rename_all = serde_rename_all(&item.attrs);
    let mut fields = Vec::new();
    for field in &named.named {
        if serde_skips_field(&field.attrs) {
            continue;
        }
        let Some(ident) = &field.ident else {
            continue;
        };
        let rust_name = ident.to_string();
        fields.push(FieldDef {
            toml_name: serde_rename(&field.attrs)
                .unwrap_or_else(|| render_field_name(&rust_name, rename_all.as_deref())),
            rust_name,
            rust_type: field.ty.to_token_stream().to_string().replace(' ', ""),
            doc: doc_text(&field.attrs),
        });
    }

    Ok(StructDef {
        doc: doc_text(&item.attrs),
        fields,
    })
}

fn parse_enum(item: &ItemEnum) -> EnumDef {
    let rename_all = serde_rename_all(&item.attrs);
    let variants = item
        .variants
        .iter()
        .map(|variant| render_variant_name(variant, rename_all.as_deref()))
        .collect();
    EnumDef { variants }
}

fn render_field_name(raw: &str, rename_all: Option<&str>) -> String {
    match rename_all {
        Some("kebab-case") => raw.replace('_', "-"),
        Some("camelCase") => snake_to_camel(raw),
        Some("PascalCase") => snake_to_pascal(raw),
        Some("SCREAMING_SNAKE_CASE") => raw.to_ascii_uppercase(),
        _ => raw.to_string(),
    }
}

fn render_variant_name(variant: &Variant, rename_all: Option<&str>) -> String {
    let raw = variant.ident.to_string();
    match rename_all {
        Some("snake_case") => camel_to_snake(&raw),
        Some("kebab-case") => camel_to_snake(&raw).replace('_', "-"),
        Some("SCREAMING_SNAKE_CASE") => camel_to_snake(&raw).to_ascii_uppercase(),
        Some("camelCase") => lower_first_ascii(&raw),
        Some("PascalCase") => raw,
        _ => raw,
    }
}

fn camel_to_snake(value: &str) -> String {
    let mut rendered = String::new();
    for (index, ch) in value.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            rendered.push('_');
        }
        rendered.push(ch.to_ascii_lowercase());
    }
    rendered
}

fn snake_to_camel(value: &str) -> String {
    let mut parts = value.split('_');
    let Some(first) = parts.next() else {
        return String::new();
    };
    let mut rendered = first.to_string();
    for part in parts {
        rendered.push_str(&capitalize_ascii(part));
    }
    rendered
}

fn snake_to_pascal(value: &str) -> String {
    value
        .split('_')
        .map(capitalize_ascii)
        .collect::<Vec<_>>()
        .join("")
}

fn capitalize_ascii(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut rendered = String::new();
    rendered.push(first.to_ascii_uppercase());
    rendered.push_str(chars.as_str());
    rendered
}

fn lower_first_ascii(value: &str) -> String {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut rendered = String::new();
    rendered.push(first.to_ascii_lowercase());
    rendered.push_str(chars.as_str());
    rendered
}

fn doc_text(attrs: &[Attribute]) -> String {
    attrs
        .iter()
        .filter_map(|attr| {
            if !attr.path().is_ident("doc") {
                return None;
            }
            match &attr.meta {
                Meta::NameValue(name_value) => match &name_value.value {
                    Expr::Lit(expr_lit) => match &expr_lit.lit {
                        Lit::Str(value) => Some(value.value().trim().to_string()),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn serde_skips_field(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("serde") {
            return false;
        }
        let mut skip = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip")
                || meta.path.is_ident("skip_deserializing")
                || meta.path.is_ident("flatten")
            {
                skip = true;
            }
            Ok(())
        });
        skip
    })
}

fn serde_rename_all(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("serde") {
            return None;
        }
        let mut found = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                found = Some(lit.value());
            }
            Ok(())
        });
        found
    })
}

fn serde_rename(attrs: &[Attribute]) -> Option<String> {
    attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("serde") {
            return None;
        }
        let mut found = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value = meta.value()?;
                let lit: syn::LitStr = value.parse()?;
                found = Some(lit.value());
            }
            Ok(())
        });
        found
    })
}

fn extract_default_config(file: &syn::File) -> Result<String> {
    let item = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Const(item_const) if item_const.ident == "DEFAULT_CONFIG" => Some(item_const),
            _ => None,
        })
        .context("failed to find DEFAULT_CONFIG in src/config.rs")?;

    let Expr::Lit(expr_lit) = &*item.expr else {
        bail!("DEFAULT_CONFIG is not a string literal");
    };
    let Lit::Str(value) = &expr_lit.lit else {
        bail!("DEFAULT_CONFIG is not a string literal");
    };
    Ok(value.value())
}

fn extract_string_array_const(file: &syn::File, name: &str) -> Result<Vec<String>> {
    let item = file
        .items
        .iter()
        .find_map(|item| match item {
            Item::Const(item_const) if item_const.ident == name => Some(item_const),
            _ => None,
        })
        .with_context(|| format!("failed to find {name} in src/config.rs"))?;

    let Expr::Array(expr_array) = &*item.expr else {
        bail!("{name} is not an array literal");
    };

    expr_array
        .elems
        .iter()
        .map(|expr| {
            let Expr::Lit(expr_lit) = expr else {
                bail!("{name} contains a non-literal entry");
            };
            let Lit::Str(value) = &expr_lit.lit else {
                bail!("{name} contains a non-string entry");
            };
            Ok(value.value())
        })
        .collect()
}

fn validate_source_schema(
    docs: &DocMap,
    defaults: &Value,
    provider_names: &[String],
    builtin_schemes: &[String],
) -> Result<()> {
    validate_required_const("KNOWN_PROVIDER_NAMES", provider_names)?;
    validate_required_const("BUILTIN_COLORSCHEME_NAMES", builtin_schemes)?;
    validate_provider_defaults(defaults, provider_names)?;
    validate_colorscheme_default(defaults, builtin_schemes)?;
    validate_section_types(docs)?;
    validate_config_root_sections(docs)?;
    validate_table_sections(docs, defaults)?;
    validate_rule_sections(docs)?;
    validate_colorscheme_section(docs)?;
    Ok(())
}

fn validate_required_const(name: &str, values: &[String]) -> Result<()> {
    if values.is_empty() {
        bail!("{name} must contain at least one string");
    }
    Ok(())
}

fn validate_provider_defaults(defaults: &Value, provider_names: &[String]) -> Result<()> {
    let known = provider_names
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let provider_array_paths: &[&[&str]] = &[
        &["providers", "disabled"],
        &["ranking", "provider_order"],
        &["ranking", "empty_query_providers"],
    ];

    for path in provider_array_paths {
        let label = dotted_path(path);
        let value = lookup_default(defaults, path.iter().copied())
            .with_context(|| format!("DEFAULT_CONFIG is missing `{label}`"))?;
        let Value::Array(values) = value else {
            bail!("DEFAULT_CONFIG `{label}` must be an array");
        };
        for value in values {
            let Value::String(provider) = value else {
                bail!("DEFAULT_CONFIG `{label}` must contain only strings");
            };
            if !known.contains(provider.as_str()) {
                bail!("DEFAULT_CONFIG `{label}` references unknown provider `{provider}`");
            }
        }
    }

    Ok(())
}

fn validate_colorscheme_default(defaults: &Value, builtin_schemes: &[String]) -> Result<()> {
    let value = lookup_default(defaults, ["ui", "colorscheme"])
        .context("DEFAULT_CONFIG is missing `ui.colorscheme`")?;
    let Value::String(colorscheme) = value else {
        bail!("DEFAULT_CONFIG `ui.colorscheme` must be a string");
    };
    if colorscheme != "system" && !builtin_schemes.iter().any(|name| name == colorscheme) {
        bail!("DEFAULT_CONFIG `ui.colorscheme` references unknown colorscheme `{colorscheme}`");
    }
    Ok(())
}

fn validate_section_types(docs: &DocMap) -> Result<()> {
    for section in SECTION_ORDER {
        match section.kind {
            SectionKind::Table | SectionKind::ProviderBoosts | SectionKind::RuleArray => {
                expect_struct(docs, section.rust_type)?;
            }
            SectionKind::PluginTable | SectionKind::PluginCommands => {
                expect_struct(docs, "Config")?;
            }
            SectionKind::Colorscheme => {
                expect_struct(docs, "UiColorschemeConfig")?;
                expect_struct(docs, "UiColorOverridesConfig")?;
            }
        }
    }
    Ok(())
}

fn validate_config_root_sections(docs: &DocMap) -> Result<()> {
    let root = expect_struct(docs, "Config")?;
    let documented_roots = SECTION_ORDER
        .iter()
        .filter_map(|section| section.default_path.first().copied())
        .collect::<HashSet<_>>();

    for field in &root.fields {
        if !documented_roots.contains(field.toml_name.as_str()) {
            bail!(
                "root Config field `{}` is not represented by any generated section",
                field.toml_name
            );
        }
    }

    Ok(())
}

fn validate_table_sections(docs: &DocMap, defaults: &Value) -> Result<()> {
    for section in SECTION_ORDER
        .iter()
        .filter(|section| matches!(section.kind, SectionKind::Table))
    {
        let item = expect_struct(docs, section.rust_type)?;
        let fields = visible_fields(section, item)?;
        if fields.is_empty() {
            bail!("{} does not render any fields", section.path);
        }
        validate_omitted_fields_are_documented_elsewhere(section, item)?;

        for field in fields {
            if field.doc.trim().is_empty() {
                bail!(
                    "{} field `{}` is missing a source doc comment",
                    section.path,
                    field.rust_name
                );
            }
            if lookup_field_default(defaults, section, field).is_none()
                && fallback_default(section.path).is_none()
            {
                bail!(
                    "{} field `{}` has no DEFAULT_CONFIG value at `{}`",
                    section.path,
                    field.rust_name,
                    default_field_path(section, field)
                );
            }
        }
    }

    Ok(())
}

fn validate_omitted_fields_are_documented_elsewhere(
    section: &SectionSpec,
    item: &StructDef,
) -> Result<()> {
    for field in &item.fields {
        if should_render_field(section, field) || is_legacy_compat_field(field) {
            continue;
        }
        if !has_child_section(section, field) {
            bail!(
                "{} omits `{}` from {}; add it to this section or document it as a child section",
                section.path,
                field.rust_name,
                section.rust_type
            );
        }
    }
    Ok(())
}

fn has_child_section(parent: &SectionSpec, field: &FieldDef) -> bool {
    let child_default_path = parent
        .default_path
        .iter()
        .copied()
        .chain(std::iter::once(field.toml_name.as_str()))
        .collect::<Vec<_>>();

    SECTION_ORDER.iter().any(|section| {
        section.path != parent.path && section.default_path == child_default_path.as_slice()
    })
}

fn validate_rule_sections(docs: &DocMap) -> Result<()> {
    for section in SECTION_ORDER
        .iter()
        .filter(|section| matches!(section.kind, SectionKind::RuleArray))
    {
        let item = expect_struct(docs, section.rust_type)?;
        if item.fields.is_empty() {
            bail!("{} does not render any fields", section.path);
        }
        for field in &item.fields {
            if field.doc.trim().is_empty() {
                bail!(
                    "{} field `{}` is missing a source doc comment",
                    section.path,
                    field.rust_name
                );
            }
        }
    }
    Ok(())
}

fn validate_colorscheme_section(docs: &DocMap) -> Result<()> {
    let colorscheme = expect_struct(docs, "UiColorschemeConfig")?;
    if !colorscheme
        .fields
        .iter()
        .any(|field| field.rust_name == "base")
    {
        bail!("UiColorschemeConfig must expose a `base` field for colorscheme docs");
    }

    let overrides = expect_struct(docs, "UiColorOverridesConfig")?;
    if overrides.fields.is_empty() {
        bail!("[ui.colorschemes.<name>] does not render any override fields");
    }
    for field in &overrides.fields {
        if field.doc.trim().is_empty() {
            bail!(
                "[ui.colorschemes.<name>] field `{}` is missing a source doc comment",
                field.rust_name
            );
        }
    }

    Ok(())
}

fn expect_struct<'a>(docs: &'a DocMap, type_name: &str) -> Result<&'a StructDef> {
    match docs.get(type_name) {
        Some(DocItem::Struct(item)) => Ok(item),
        Some(DocItem::Enum(_)) => bail!("{type_name} is an enum, but generated docs need a struct"),
        None => bail!("missing doc item `{type_name}` from src/config.rs"),
    }
}

fn default_field_path(section: &SectionSpec, field: &FieldDef) -> String {
    dotted_path(
        &section
            .default_path
            .iter()
            .copied()
            .chain(std::iter::once(field.toml_name.as_str()))
            .collect::<Vec<_>>(),
    )
}

fn dotted_path(path: &[&str]) -> String {
    path.join(".")
}

fn render_document(
    docs: &DocMap,
    defaults: &Value,
    provider_names: &[String],
    builtin_schemes: &[String],
) -> Result<String> {
    let enum_values = enum_values_map(docs);
    let mut out = String::new();
    out.push_str("<!-- Generated by scripts/generate-config-docs.sh. Do not edit by hand. -->\n");
    out.push_str("# Configuration\n\n");
    out.push_str(
        "Runx reads `~/Library/Application Support/runx/config.toml`. The file is created on first launch, validated on load, and reloaded when you open Runx after the file changes.\n\n",
    );
    out.push_str(
        "This reference is generated from `src/config.rs`, so the documented keys, defaults, and allowed values stay tied to the real code.\n\n",
    );
    out.push_str("Most users only touch a few sections:\n\n");
    out.push_str("- `[hotkey]` for the launcher shortcut\n");
    out.push_str("- `[window]` for placement and basic launcher behavior\n");
    out.push_str("- `[providers]` to disable built-in providers\n");
    out.push_str("- `[providers.windows]` and `[providers.apps]` for provider-specific behavior\n");
    out.push_str("- `[ranking]` for provider order and empty-query behavior\n");
    out.push_str("- `[ui]` and `[ui.colorschemes.<name>]` for appearance\n");
    out.push_str("- `[plugin.<id>]` only when a plugin needs configuration\n\n");

    for section in SECTION_ORDER {
        match section.kind {
            SectionKind::Table => {
                let item = docs
                    .get(section.rust_type)
                    .with_context(|| format!("missing doc item `{}`", section.rust_type))?;
                let DocItem::Struct(struct_def) = item else {
                    bail!("{} is not a struct", section.rust_type);
                };
                out.push_str(&render_table_section(
                    section,
                    struct_def,
                    defaults,
                    provider_names,
                    builtin_schemes,
                    &enum_values,
                )?);
            }
            SectionKind::ProviderBoosts => {
                out.push_str(&render_provider_boosts_section(section, provider_names));
            }
            SectionKind::RuleArray => {
                let item = docs
                    .get(section.rust_type)
                    .with_context(|| format!("missing doc item `{}`", section.rust_type))?;
                let DocItem::Struct(struct_def) = item else {
                    bail!("{} is not a struct", section.rust_type);
                };
                out.push_str(&render_rule_array_section(
                    section,
                    struct_def,
                    provider_names,
                    &enum_values,
                )?);
            }
            SectionKind::PluginTable => out.push_str(&render_plugin_table_section(section)),
            SectionKind::PluginCommands => out.push_str(&render_plugin_commands_section(section)),
            SectionKind::Colorscheme => {
                let item = docs
                    .get("UiColorOverridesConfig")
                    .context("missing UiColorOverridesConfig docs")?;
                let DocItem::Struct(overrides) = item else {
                    bail!("UiColorOverridesConfig is not a struct");
                };
                out.push_str(&render_colorscheme_section(
                    section,
                    overrides,
                    builtin_schemes,
                ));
            }
        }
    }

    out.push_str("## Legacy Compatibility\n\n");
    out.push_str(
        "Runx still accepts the older light/dark color compatibility keys under `[ui]`, `[ui.colors]`, and `[ui.dark_colors]`, but new configs should prefer `[ui.colorschemes.<name>]`.\n",
    );
    Ok(out)
}

fn render_table_section(
    section: &SectionSpec,
    item: &StructDef,
    defaults: &Value,
    provider_names: &[String],
    builtin_schemes: &[String],
    enum_values: &HashMap<String, Vec<String>>,
) -> Result<String> {
    let mut rows = Vec::new();

    for field in visible_fields(section, item)? {
        let default = lookup_field_default(defaults, section, field);
        rows.push((
            format!("`{}`", field.toml_name),
            describe_type(
                section.path,
                field,
                provider_names,
                builtin_schemes,
                enum_values,
            ),
            format_default(default, fallback_default(section.path)),
            field.doc.clone(),
        ));
    }

    Ok(render_section(section.path, &item.doc, &rows))
}

fn render_provider_boosts_section(section: &SectionSpec, provider_names: &[String]) -> String {
    let intro = "Per-provider additive score boosts. Use provider names as keys. Omitted providers behave as if their boost were `0`.";
    let rows = vec![(
        "Provider key".to_string(),
        "integer".to_string(),
        "`0` when omitted".to_string(),
        provider_names
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", "),
    )];
    render_section(section.path, intro, &rows)
}

fn render_rule_array_section(
    section: &SectionSpec,
    item: &StructDef,
    provider_names: &[String],
    enum_values: &HashMap<String, Vec<String>>,
) -> Result<String> {
    let mut rows = Vec::new();
    for field in &item.fields {
        if field.doc.trim().is_empty() {
            bail!(
                "{} field `{}` is missing a source doc comment",
                section.path,
                field.rust_name
            );
        }
        let type_desc = if is_provider_name_array_field(&field.rust_name) {
            format!("array of provider names ({})", provider_names.join(", "))
        } else {
            describe_type(section.path, field, provider_names, &[], enum_values)
        };
        rows.push((
            format!("`{}`", field.toml_name),
            type_desc,
            "-".to_string(),
            field.doc.clone(),
        ));
    }
    Ok(render_section(section.path, &item.doc, &rows))
}

fn render_plugin_table_section(section: &SectionSpec) -> String {
    let intro = "Per-plugin configuration lives under `[plugin.<id>]`. Runx passes the full table to the plugin runtime as `runx.plugin_config`, except for the reserved `commands` subtable.";
    let rows = vec![
        (
            "`commands`".to_string(),
            "table".to_string(),
            "-".to_string(),
            "Reserved for command routing. See `[plugin.<id>.commands]`.".to_string(),
        ),
        (
            "`<your keys>`".to_string(),
            "TOML values".to_string(),
            "-".to_string(),
            "Arbitrary plugin-specific settings. These are available inside the plugin through `runx.plugin_config`.".to_string(),
        ),
    ];
    render_section(section.path, intro, &rows)
}

fn render_plugin_commands_section(section: &SectionSpec) -> String {
    let intro = "Command routing maps a typed prefix to a named Lua search handler. A route matches when the query is exactly the command or starts with the command followed by a space. Once a plugin has any configured commands, Runx stops calling its generic `search(query)` function for unrelated queries. Routed plugins are routed-only.";
    let rows = vec![
        (
            "`<command>`".to_string(),
            "string key".to_string(),
            "-".to_string(),
            "Command prefix typed into Runx, for example `pass` or `emoji`.".to_string(),
        ),
        (
            "value".to_string(),
            "string".to_string(),
            "-".to_string(),
            "Name of the exported Lua handler to call, for example `search_type_password`."
                .to_string(),
        ),
    ];
    render_section(section.path, intro, &rows)
}

fn render_colorscheme_section(
    section: &SectionSpec,
    overrides: &StructDef,
    builtin_schemes: &[String],
) -> String {
    let intro = format!(
        "Named colorscheme definitions. Runx always knows about {}. Custom schemes can use any other name and are selected through `[ui].colorscheme`. Omitted keys inherit from the base palette or the built-in default. Most users only need `accent`, `background`, `panel`, `text`, and `muted`; the remaining keys are lower-level UI tokens for precise theme work.",
        builtin_schemes
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let mut rows = vec![(
        "`base`".to_string(),
        "one of: `builtin_light`, `builtin_dark`".to_string(),
        "Base palette inherited by a custom scheme. Built-in schemes must not set this."
            .to_string(),
    )];
    rows.extend(overrides.fields.iter().map(|field| {
        (
            format!("`{}`", field.toml_name),
            "string".to_string(),
            field.doc.clone(),
        )
    }));
    render_section_without_defaults(section.path, &intro, &rows)
}

fn render_section(path: &str, intro: &str, rows: &[(String, String, String, String)]) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {path}\n\n"));
    out.push_str(intro);
    out.push_str("\n\n");
    out.push_str("| Option | Type | Default | Description |\n");
    out.push_str("| --- | --- | --- | --- |\n");
    for (option, type_desc, default, description) in rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            option,
            escape_pipes(type_desc),
            escape_pipes(default),
            escape_pipes(description)
        ));
    }
    out.push('\n');
    out
}

fn render_section_without_defaults(
    path: &str,
    intro: &str,
    rows: &[(String, String, String)],
) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {path}\n\n"));
    out.push_str(intro);
    out.push_str("\n\n");
    out.push_str("| Option | Type | Description |\n");
    out.push_str("| --- | --- | --- |\n");
    for (option, type_desc, description) in rows {
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            option,
            escape_pipes(type_desc),
            escape_pipes(description)
        ));
    }
    out.push('\n');
    out
}

fn field_filter(path: &str) -> Option<&'static [&'static str]> {
    match path {
        "[providers]" => Some(&["disabled"]),
        "[ranking]" => Some(&[
            "tie_threshold",
            "provider_order",
            "empty_query_providers",
            "result_limit",
        ]),
        "[ui]" => Some(&[
            "show_header",
            "cycle_selection",
            "colorscheme",
            "font_family",
        ]),
        _ => None,
    }
}

fn visible_fields<'a>(section: &SectionSpec, item: &'a StructDef) -> Result<Vec<&'a FieldDef>> {
    validate_field_filter(section, item)?;
    Ok(item
        .fields
        .iter()
        .filter(|field| should_render_field(section, field))
        .collect())
}

fn validate_field_filter(section: &SectionSpec, item: &StructDef) -> Result<()> {
    let Some(allowed) = field_filter(section.path) else {
        return Ok(());
    };

    let actual = item
        .fields
        .iter()
        .map(|field| field.rust_name.as_str())
        .collect::<HashSet<_>>();

    for field_name in allowed {
        if !actual.contains(field_name) {
            bail!(
                "{} field filter references missing `{}` on {}",
                section.path,
                field_name,
                section.rust_type
            );
        }
    }

    Ok(())
}

fn should_render_field(section: &SectionSpec, field: &FieldDef) -> bool {
    if let Some(allowed) = field_filter(section.path)
        && !allowed.contains(&field.rust_name.as_str())
    {
        return false;
    }
    if is_legacy_compat_field(field) {
        return false;
    }
    if section.path == "[ui]" && is_ui_nested_table_field(field) {
        return false;
    }
    true
}

fn is_legacy_compat_field(field: &FieldDef) -> bool {
    field.doc.starts_with("Legacy compatibility alias")
}

fn is_ui_nested_table_field(field: &FieldDef) -> bool {
    matches!(
        field.rust_name.as_str(),
        "colorschemes" | "canvas" | "entries" | "shortcuts" | "font_sizes" | "layout"
    )
}

fn lookup_default<'v, 'p>(
    value: &'v Value,
    path: impl IntoIterator<Item = &'p str>,
) -> Option<&'v Value> {
    let mut current = value;
    for key in path {
        current = current.get(key)?;
    }
    Some(current)
}

fn lookup_field_default<'a>(
    defaults: &'a Value,
    section: &SectionSpec,
    field: &FieldDef,
) -> Option<&'a Value> {
    lookup_default(
        defaults,
        section
            .default_path
            .iter()
            .copied()
            .chain(std::iter::once(field.toml_name.as_str())),
    )
}

fn fallback_default(path: &str) -> Option<&'static str> {
    match path {
        "[ui.colorschemes.<name>]" => Some("unset"),
        _ => None,
    }
}

fn format_default(value: Option<&Value>, fallback: Option<&str>) -> String {
    match value {
        Some(Value::String(value)) => format!("`{:?}`", value),
        Some(Value::Integer(value)) => format!("`{value}`"),
        Some(Value::Float(value)) => format!("`{value}`"),
        Some(Value::Boolean(value)) => format!("`{value}`"),
        Some(Value::Array(values)) => {
            let rendered = values
                .iter()
                .map(|value| match value {
                    Value::String(value) => format!("{:?}", value),
                    Value::Integer(value) => value.to_string(),
                    Value::Float(value) => value.to_string(),
                    Value::Boolean(value) => value.to_string(),
                    _ => "table".to_string(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("`[{rendered}]`")
        }
        Some(_) => "`table`".to_string(),
        None => fallback
            .map(|value| format!("`{value}`"))
            .unwrap_or_else(|| "-".to_string()),
    }
}

fn describe_type(
    section_path: &str,
    field: &FieldDef,
    provider_names: &[String],
    builtin_schemes: &[String],
    enum_values: &HashMap<String, Vec<String>>,
) -> String {
    match (section_path, field.rust_name.as_str()) {
        ("[hotkey]", "key") => {
            "string (`A-Z`, `0-9`, `Space`, `Enter`, `Escape`, `Tab`, `Backspace`, arrows)"
                .to_string()
        }
        ("[ui.shortcuts]", _) if is_ui_shortcut_field(field) => {
            "shortcut string such as `Enter` or `Option+Enter`, or `none`".to_string()
        },
        (_, "modifiers") => "array of strings (`Alt`, `Option`, `Control`, `Ctrl`, `Shift`, `Command`, `Cmd`, `Super`, `Meta`)".to_string(),
        (_, field_name) if is_provider_name_array_field(field_name) => {
            format!(
                "array of provider names ({})",
                provider_names
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        (_, "colorscheme") => {
            let values = std::iter::once("system".to_string())
                .chain(builtin_schemes.iter().cloned())
                .map(|value| format!("`{value}`"))
                .collect::<Vec<_>>()
                .join(", ");
            format!("string ({values}, or a custom name under `[ui.colorschemes.<name>]`)")
        }
        _ => {
            if let Some(values) = enum_values.get(&field.rust_type) {
                return format!(
                    "one of: {}",
                    values
                        .iter()
                        .map(|value| format!("`{value}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            if let Some(inner) = field.rust_type.strip_prefix("Vec<").and_then(|value| value.strip_suffix('>')) {
                return format!("array of {}", simple_type_name(inner));
            }
            if field.rust_type.starts_with("HashMap<String,") {
                return "TOML table".to_string();
            }
            simple_type_name(&field.rust_type)
        }
    }
}

fn enum_values_map(docs: &DocMap) -> HashMap<String, Vec<String>> {
    docs.iter()
        .filter_map(|(name, item)| match item {
            DocItem::Enum(definition) => Some((name.clone(), definition.variants.clone())),
            DocItem::Struct(_) => None,
        })
        .collect()
}

fn is_ui_shortcut_field(field: &FieldDef) -> bool {
    field.rust_type == "Option<UiShortcutConfig>"
}

fn is_provider_name_array_field(field_name: &str) -> bool {
    matches!(
        field_name,
        "provider_order" | "empty_query_providers" | "providers" | "disabled"
    )
}

fn simple_type_name(rust_type: &str) -> String {
    match rust_type {
        "String" => "string".to_string(),
        "bool" => "boolean".to_string(),
        "u16" | "u32" | "u64" | "usize" | "i64" => "integer".to_string(),
        "f64" => "number".to_string(),
        "UiCanvasConfig"
        | "UiEntriesConfig"
        | "UiShortcutsConfig"
        | "UiFontSizesConfig"
        | "UiLayoutConfig"
        | "UiColorschemeConfig"
        | "WindowsProviderConfig"
        | "AppsProviderConfig" => "table".to_string(),
        other => format!("`{other}`"),
    }
}

fn escape_pipes(value: &str) -> String {
    value.replace('|', "\\|")
}
