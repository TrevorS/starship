use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[cfg_attr(
    feature = "config-schema",
    derive(schemars::JsonSchema),
    schemars(deny_unknown_fields)
)]
#[serde(default)]
pub struct JjStatusConfig<'a> {
    pub symbol: &'a str,
    pub style: &'a str,
    pub format: &'a str,
    pub truncation_length: i64,
    pub truncation_symbol: &'a str,
    pub conflicted: &'a str,
    pub divergent: &'a str,
    pub hidden: &'a str,
    pub ignore_working_copy: bool,
    pub disabled: bool,
}

impl Default for JjStatusConfig<'_> {
    fn default() -> Self {
        Self {
            symbol: "jj ",
            style: "bold purple",
            format: "[$symbol$change_id]($style)( [$bookmarks]($style))([ $conflicted$divergent$hidden]($style)) ",
            truncation_length: 8,
            truncation_symbol: "…",
            conflicted: "×",
            divergent: "?",
            hidden: "◌",
            ignore_working_copy: true,
            disabled: true,
        }
    }
}
