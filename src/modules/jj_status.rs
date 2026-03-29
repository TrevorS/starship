use super::utils::truncate::truncate_text;
use super::{Context, Module, ModuleConfig, vcs};

use crate::configs::jj_status::JjStatusConfig;
use crate::formatter::StringFormatter;

// Template used to query jj for status information.
// Note: We use the full change_id and truncate in Rust for user-configurable length.
// Bookmarks are space-separated for display. If a bookmark name contains spaces,
// it will appear as multiple words (this is a display limitation, not a parsing error).
const JJ_TEMPLATE: &str =
    r#"change_id ++ "\n" ++ local_bookmarks.join(" ") ++ "\n" ++ conflict ++ "\n" ++ divergent ++ "\n" ++ hidden"#;

/// Creates a module with the Jujutsu status in the current directory
///
/// Will display the current change ID, bookmarks, and status if the current directory is a jj repo
pub fn module<'a>(context: &'a Context) -> Option<Module<'a>> {
    let mut module = context.new_module("jj_status");
    let config: JjStatusConfig = JjStatusConfig::try_load(module.config);

    // We default to disabled=true, so we have to check after loading our config module.
    if config.disabled {
        return None;
    }

    vcs::discover_repo_root(context, vcs::Vcs::Jujutsu)?;

    let len = if config.truncation_length <= 0 {
        log::warn!(
            "\"truncation_length\" should be a positive value, found {}",
            config.truncation_length
        );
        usize::MAX
    } else {
        config.truncation_length as usize
    };

    let jj_info = get_jj_info(context, &config)?;

    let change_id = truncate_text(&jj_info.change_id, len, config.truncation_symbol);

    let bookmarks = (!jj_info.bookmarks.is_empty()).then_some(jj_info.bookmarks);
    let conflicted = jj_info.conflicted.then_some(config.conflicted);
    let divergent = jj_info.divergent.then_some(config.divergent);
    let hidden = jj_info.hidden.then_some(config.hidden);

    let parsed = StringFormatter::new(config.format).and_then(|formatter| {
        formatter
            .map_meta(|variable, _| match variable {
                "symbol" => Some(config.symbol),
                _ => None,
            })
            .map_style(|variable| match variable {
                "style" => Some(Ok(config.style)),
                _ => None,
            })
            .map(|variable| match variable {
                "change_id" => Some(Ok(change_id.as_str())),
                "bookmarks" => bookmarks.as_deref().map(Ok),
                "conflicted" => conflicted.map(Ok),
                "divergent" => divergent.map(Ok),
                "hidden" => hidden.map(Ok),
                _ => None,
            })
            .parse(None, Some(context))
    });

    module.set_segments(match parsed {
        Ok(segments) => segments,
        Err(error) => {
            log::warn!("Error in module `jj_status`:\n{error}");
            return None;
        }
    });

    Some(module)
}

struct JjInfo {
    change_id: String,
    bookmarks: String,
    conflicted: bool,
    divergent: bool,
    hidden: bool,
}

// Parse a boolean value from jj output, warning on unexpected values.
fn parse_jj_bool(value: Option<&str>, field_name: &str) -> bool {
    match value {
        Some("true") => true,
        Some("false") | None => false,
        Some(other) => {
            log::warn!(
                "jj_status: unexpected value for {}: {:?}, treating as false",
                field_name,
                other
            );
            false
        }
    }
}

fn get_jj_info(ctx: &Context, config: &JjStatusConfig) -> Option<JjInfo> {
    // Build command args based on config
    let mut args = vec!["log", "--no-graph", "-r", "@"];

    // --ignore-working-copy skips auto-snapshotting for faster prompts,
    // but may show slightly stale data. Configurable for users who prefer accuracy.
    if config.ignore_working_copy {
        args.push("--ignore-working-copy");
    }

    args.push("-T");
    args.push(JJ_TEMPLATE);

    let output = ctx.exec_cmd("jj", &args)?.stdout;

    let mut lines = output.lines();

    let change_id = match lines.next() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            log::warn!("jj_status: unexpected output from jj log - missing change_id");
            log::debug!("jj_status: raw output was: {:?}", output);
            return None;
        }
    };

    let bookmarks = lines.next().unwrap_or("").to_string();
    let conflicted = parse_jj_bool(lines.next(), "conflict");
    let divergent = parse_jj_bool(lines.next(), "divergent");
    let hidden = parse_jj_bool(lines.next(), "hidden");

    Some(JjInfo {
        change_id,
        bookmarks,
        conflicted,
        divergent,
        hidden,
    })
}

#[cfg(test)]
mod tests {
    use super::JJ_TEMPLATE;
    use nu_ansi_term::Color;
    use std::io;

    use crate::test::{FixtureProvider, ModuleRenderer, fixture_repo};
    use crate::utils::CommandOutput;

    // Realistic 32-char change ID for testing truncation behavior
    const TEST_CHANGE_ID: &str = "kxmynpvqwlszotryrmlqkpqrztnlmosn";

    // Builds the expected command string for test mocking
    fn jj_cmd(ignore_working_copy: bool) -> String {
        let base = "jj log --no-graph -r @";
        let iwc = if ignore_working_copy {
            " --ignore-working-copy"
        } else {
            ""
        };
        format!("{base}{iwc} -T {JJ_TEMPLATE}")
    }

    fn mock_jj_output(change_id: &str, bookmarks: &str, conflict: bool, divergent: bool, hidden: bool) -> CommandOutput {
        CommandOutput {
            stdout: format!(
                "{}\n{}\n{}\n{}\n{}",
                change_id,
                bookmarks,
                conflict,
                divergent,
                hidden
            ),
            stderr: String::default(),
        }
    }

    #[test]
    fn test_jj_nothing_on_empty_dir() -> io::Result<()> {
        let repo_dir = tempfile::tempdir()?;

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir.path())
            .collect();

        let expected = None;
        assert_eq!(expected, actual);
        repo_dir.close()
    }

    #[test]
    fn test_jj_disabled_per_default() -> io::Result<()> {
        // Module is disabled by default, so even with a .jj dir it should return None
        // when no explicit disabled=false is set
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                truncation_length = 14
                // Note: disabled is NOT set to false, so it defaults to true
            })
            .collect();

        assert_eq!(None, actual);
        tempdir.close()
    }

    #[test]
    fn test_jj_no_repo() -> io::Result<()> {
        // No .jj directory means module returns None even when enabled
        let tempdir = tempfile::tempdir()?;

        let actual = ModuleRenderer::new("jj_status")
            .path(tempdir.path())
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            .collect();

        assert_eq!(None, actual);
        tempdir.close()
    }

    #[test]
    fn test_jj_status_simple() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(TEST_CHANGE_ID, "main", false, false, false)))
            .collect();

        // Default truncation_length=8, so 32-char ID becomes "kxmynpvq…"
        let expected = Some(format!(
            "{} {} ",
            Color::Purple.bold().paint("jj kxmynpvq…"),
            Color::Purple.bold().paint("main")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_status_no_bookmarks() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(TEST_CHANGE_ID, "", false, false, false)))
            .collect();

        // With no bookmarks, format shows just change_id (bookmarks group is conditional)
        let expected = Some(format!(
            "{} ",
            Color::Purple.bold().paint("jj kxmynpvq…")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_status_conflicted() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(TEST_CHANGE_ID, "", true, false, false)))
            .collect();

        let expected = Some(format!(
            "{} ",
            Color::Purple.bold().paint("jj kxmynpvq… ×")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_status_divergent() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(TEST_CHANGE_ID, "", false, true, false)))
            .collect();

        let expected = Some(format!(
            "{} ",
            Color::Purple.bold().paint("jj kxmynpvq… ?")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_ignore_working_copy_disabled() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
                ignore_working_copy = false
            })
            // When ignore_working_copy is false, the flag should not be in the command
            .cmd(&jj_cmd(false), Some(mock_jj_output(TEST_CHANGE_ID, "main", false, false, false)))
            .collect();

        let expected = Some(format!(
            "{} {} ",
            Color::Purple.bold().paint("jj kxmynpvq…"),
            Color::Purple.bold().paint("main")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_configured() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                style = "underline blue"
                symbol = "J "
                truncation_length = 4
                truncation_symbol = "%"
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(TEST_CHANGE_ID, "main", false, false, false)))
            .collect();

        let expected = Some(format!(
            "{} {} ",
            Color::Blue.underline().paint("J kxmy%"),
            Color::Blue.underline().paint("main")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_multiple_bookmarks() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(TEST_CHANGE_ID, "main feature", false, false, false)))
            .collect();

        let expected = Some(format!(
            "{} {} ",
            Color::Purple.bold().paint("jj kxmynpvq…"),
            Color::Purple.bold().paint("main feature")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_status_hidden() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(TEST_CHANGE_ID, "", false, false, true)))
            .collect();

        let expected = Some(format!(
            "{} ",
            Color::Purple.bold().paint("jj kxmynpvq… ◌")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_status_combined_flags() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        // Test with both conflicted AND divergent set
        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(TEST_CHANGE_ID, "main", true, true, false)))
            .collect();

        // Adjacent styled segments with same style get combined:
        // styled(change_id) + " " + styled(bookmarks + " " + flags) + " "
        let expected = Some(format!(
            "{} {} ",
            Color::Purple.bold().paint("jj kxmynpvq…"),
            Color::Purple.bold().paint("main ×?")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_default_truncation_symbol() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        // Use a longer change_id to trigger truncation with default "…" symbol
        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                truncation_length = 4
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output("kxmynpvqwlsz", "", false, false, false)))
            .collect();

        // Default truncation_symbol is "…"
        let expected = Some(format!(
            "{} ",
            Color::Purple.bold().paint("jj kxmy…")
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_truncation_length_zero_uses_max() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        // With truncation_length = 0, should use usize::MAX (no truncation)
        let long_id = "kxmynpvqwlszabcdefgh";
        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                truncation_length = 0
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(long_id, "", false, false, false)))
            .collect();

        // No truncation should occur
        let expected = Some(format!(
            "{} ",
            Color::Purple.bold().paint(format!("jj {long_id}"))
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_truncation_length_negative_uses_max() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        // With truncation_length = -1, should also use usize::MAX (no truncation)
        let long_id = "kxmynpvqwlszabcdefgh";
        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                truncation_length = -1
                disabled = false
            })
            .cmd(&jj_cmd(true), Some(mock_jj_output(long_id, "", false, false, false)))
            .collect();

        // No truncation should occur (negative treated same as zero)
        let expected = Some(format!(
            "{} ",
            Color::Purple.bold().paint(format!("jj {long_id}"))
        ));
        assert_eq!(expected, actual);

        tempdir.close()
    }

    #[test]
    fn test_jj_command_failure() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        // When jj command fails (returns None), module should return None
        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            // Don't mock the command - it will fail since jj isn't installed
            .collect();

        assert_eq!(None, actual);
        tempdir.close()
    }

    #[test]
    fn test_jj_malformed_output() -> io::Result<()> {
        let tempdir = fixture_repo(FixtureProvider::Jujutsu)?;
        let repo_dir = tempdir.path();

        // When jj returns empty/malformed output, module should return None
        let actual = ModuleRenderer::new("jj_status")
            .path(repo_dir)
            .config(toml::toml! {
                [jj_status]
                disabled = false
            })
            .cmd(
                &jj_cmd(true),
                Some(CommandOutput {
                    stdout: String::new(), // Empty output
                    stderr: String::default(),
                }),
            )
            .collect();

        assert_eq!(None, actual);
        tempdir.close()
    }
}
