//! `[accounts.NAME]`, `[account_env.NAME]` and an agent's `account`: what loads and every way they are refused.
use super::{
    AccountError, AccountName, AccountSecretsError, Config, ConfigError, ContextKey, EnvValue, OwnerName, Secrets,
};
use crate::config::ValueRule;

/// A secret that must never show in an error.
const MARKER: &str = "MARKER-7f3a";

/// The mode of a file only its owner can use.
const OWNER_ONLY: u32 = 0o600;

fn account(text: &str) -> AccountName {
    text.parse().unwrap()
}

/// A configuration with an account `comp-claude` of owner `comp`, an agent and an investigator that use it.
fn with_account(extra: &str) -> String {
    format!(
        r#"
[accounts.comp-claude]
owner = "comp"
{extra}

[agents.comp-claude-1]
cli = "claude"
account = "comp-claude"

[investigators.comp-inv-opus]
cli = "claude"
model = "opus"
account = "comp-claude"

[tiers]
normal = ["comp-claude-1"]
"#
    )
}

/// A configuration with the account `comp-claude` of owner `comp` and one investigator named `name` that uses it.
fn named_investigator(name: &str) -> String {
    format!(
        r#"
[accounts.comp-claude]
owner = "comp"

[investigators.{name}]
cli = "claude"
account = "comp-claude"
"#
    )
}

fn load(text: &str) -> ConfigError {
    Config::from_toml_str(text).unwrap_err()
}

#[test]
fn account_with_env_loads() {
    let config = Config::from_toml_str(&with_account(
        "[accounts.comp-claude.env]\nCLAUDE_CONFIG_DIR = \"/home/x/.claude-comp\"",
    ))
    .unwrap();

    let found = &config.accounts()[&account("comp-claude")];
    assert_eq!(found.owner(), &"comp".parse::<OwnerName>().unwrap());
    let env: Vec<_> = found.env().iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    assert_eq!(env, [("CLAUDE_CONFIG_DIR", "/home/x/.claude-comp")]);
    let agent = &config.agents()[&"comp-claude-1".parse().unwrap()];
    assert_eq!(agent.account, Some(account("comp-claude")));
    let investigator = &config.investigators()[&"comp-inv-opus".parse().unwrap()];
    assert_eq!(investigator.account, Some(account("comp-claude")));
}

#[test]
fn account_without_env_loads() {
    let config = Config::from_toml_str(&with_account("")).unwrap();

    assert!(config.accounts()[&account("comp-claude")].env().is_empty());
}

#[test]
fn agent_without_account_loads_as_today() {
    let config = Config::from_toml_str(
        "[agents.codex-1]\ncli = \"codex\"\n[investigators.inv-opus]\ncli = \"claude\"\n[tiers]\nnormal = [\"codex-1\"]\n",
    )
    .unwrap();

    assert!(config.accounts().is_empty());
    assert_eq!(config.agents()[&"codex-1".parse().unwrap()].account, None);
    assert_eq!(config.investigators()[&"inv-opus".parse().unwrap()].account, None);
}

#[test]
fn agent_names_missing_account() {
    let text =
        "[agents.comp-claude-1]\ncli = \"claude\"\naccount = \"comp-claude\"\n[tiers]\nnormal = [\"comp-claude-1\"]\n";

    assert!(matches!(
        load(text),
        ConfigError::Account(AccountError::UnknownAccount { key: ContextKey::Agent, name, account: a })
            if name == "comp-claude-1" && a == account("comp-claude")
    ));
}

#[test]
fn investigator_names_missing_account() {
    let text = "[investigators.comp-inv]\ncli = \"claude\"\naccount = \"comp-claude\"\n";

    assert!(matches!(
        load(text),
        ConfigError::Account(AccountError::UnknownAccount { key: ContextKey::Investigator, name, account: a })
            if name == "comp-inv" && a == account("comp-claude")
    ));
}

#[test]
fn account_without_owner() {
    let text = "[accounts.comp-claude]\n[investigators.comp-inv]\ncli = \"claude\"\naccount = \"comp-claude\"\n";

    assert!(matches!(
        load(text),
        ConfigError::Account(AccountError::OwnerMissing { account: a }) if a == account("comp-claude")
    ));
}

#[test]
fn account_with_empty_owner() {
    let text =
        "[accounts.comp-claude]\nowner = \"\"\n[investigators.comp-inv]\ncli = \"claude\"\naccount = \"comp-claude\"\n";

    assert!(matches!(
        load(text),
        ConfigError::Account(AccountError::OwnerInvalid { account: a, source })
            if a == account("comp-claude") && source.rule == ValueRule::Empty
    ));
}

#[test]
fn account_with_owner_comp_x() {
    let text = "[accounts.comp-claude]\nowner = \"Comp X\"\n[investigators.comp-inv]\ncli = \"claude\"\naccount = \"comp-claude\"\n";

    assert!(matches!(
        load(text),
        ConfigError::Account(AccountError::OwnerInvalid { account: a, source })
            if a == account("comp-claude") && source.value == "Comp X" && source.rule == ValueRule::ForbiddenChar('C')
    ));
}

#[test]
fn account_with_owner_starting_with_a_digit_is_refused() {
    let text = "[accounts.comp-claude]\nowner = \"1comp\"\n[investigators.comp-inv]\ncli = \"claude\"\naccount = \"comp-claude\"\n";

    assert!(matches!(
        load(text),
        ConfigError::Account(AccountError::OwnerInvalid { source, .. }) if source.rule == ValueRule::StartsWithDigit
    ));
}

#[test]
fn account_no_agent_names() {
    let text = "[accounts.comp-claude]\nowner = \"comp\"\n";

    assert!(matches!(
        load(text),
        ConfigError::Account(AccountError::Unused { account: a }) if a == account("comp-claude")
    ));
}

#[test]
fn max_runs_and_hold_at_are_unknown_keys_until_their_items() {
    for key in ["max_runs = 2", "hold_at = 0.9"] {
        let text = format!("[accounts.comp-claude]\nowner = \"comp\"\n{key}\n");
        assert!(matches!(load(&text), ConfigError::Parse(_)), "{key}");
    }
}

#[test]
fn variable_in_env_and_account_env() {
    let config =
        Config::from_toml_str(&with_account("[accounts.comp-claude.env]\nCLAUDE_CONFIG_DIR = \"/x\"")).unwrap();
    let secrets = Secrets::parse(
        &format!("[account_env.comp-claude]\nCLAUDE_CONFIG_DIR = \"{MARKER}\"\n"),
        OWNER_ONLY,
    )
    .unwrap();

    assert_eq!(
        config.check_account_env(&secrets),
        Err(AccountSecretsError::VariableTwice {
            account: account("comp-claude"),
            variable: "CLAUDE_CONFIG_DIR".parse().unwrap(),
        })
    );
}

#[test]
fn account_env_with_other_variables_is_accepted() {
    let config =
        Config::from_toml_str(&with_account("[accounts.comp-claude.env]\nCLAUDE_CONFIG_DIR = \"/x\"")).unwrap();
    let secrets = Secrets::parse(
        &format!("[account_env.comp-claude]\nCLAUDE_CODE_OAUTH_TOKEN = \"{MARKER}\"\n"),
        OWNER_ONLY,
    )
    .unwrap();

    assert_eq!(config.check_account_env(&secrets), Ok(()));
    let table = &secrets.account_env()[&account("comp-claude")];
    assert_eq!(table[&"CLAUDE_CODE_OAUTH_TOKEN".parse().unwrap()].expose(), MARKER);
}

#[test]
fn account_env_without_account() {
    let config = Config::from_toml_str(&with_account("")).unwrap();
    let secrets = Secrets::parse(&format!("[account_env.other]\nTOKEN = \"{MARKER}\"\n"), OWNER_ONLY).unwrap();

    assert_eq!(
        config.check_account_env(&secrets),
        Err(AccountSecretsError::NoAccount {
            account: account("other")
        })
    );
}

#[test]
fn secrets_without_account_env_fit_any_configuration() {
    let secrets = Secrets::parse("github_token = \"ghp_x\"\n", OWNER_ONLY).unwrap();

    assert_eq!(
        Config::from_toml_str(&with_account(""))
            .unwrap()
            .check_account_env(&secrets),
        Ok(())
    );
    assert_eq!(Config::from_toml_str("").unwrap().check_account_env(&secrets), Ok(()));
}

#[test]
fn comp_inv_opus_loads() {
    let config = Config::from_toml_str(&named_investigator("comp-inv-opus")).unwrap();

    assert_eq!(config.investigators().len(), 1);
}

#[test]
fn name_rule_refuses_a_name_without_its_owner() {
    for name in ["inv-opus", "mateusz-inv-opus", "compx-inv-opus", "comp"] {
        assert!(
            matches!(
                load(&named_investigator(name)),
                ConfigError::Account(AccountError::NameWithoutOwner { key: ContextKey::Investigator, name: n, account: a, owner })
                    if n == name && a == account("comp-claude") && owner == "comp".parse().unwrap()
            ),
            "{name}"
        );
    }
}

#[test]
fn inv_opus_is_refused_for_an_agent_too() {
    let text = "[accounts.comp-claude]\nowner = \"comp\"\n[agents.claude-1]\ncli = \"claude\"\naccount = \"comp-claude\"\n[tiers]\nnormal = [\"claude-1\"]\n";

    assert!(matches!(
        load(text),
        ConfigError::Account(AccountError::NameWithoutOwner { key: ContextKey::Agent, name, .. }) if name == "claude-1"
    ));
}

#[test]
fn agent_without_account_any_name_loads() {
    let text = "[accounts.comp-claude]\nowner = \"comp\"\n[agents.mateusz-1]\ncli = \"claude\"\n[agents.comp-2]\ncli = \"claude\"\naccount = \"comp-claude\"\n[investigators.inv-opus]\ncli = \"claude\"\n[tiers]\nnormal = [\"mateusz-1\", \"comp-2\"]\n";
    // mateusz-1 has no account, so any name goes; comp-2 starts with its owner and a hyphen.
    let config = Config::from_toml_str(text).unwrap();

    assert_eq!(config.agents().len(), 2);
}

#[test]
fn account_names_are_checked() {
    for bad in ["Comp", "1comp", "-comp", "comp-", "comp x", ""] {
        let text = format!("[accounts.\"{bad}\"]\nowner = \"comp\"\n");
        assert!(matches!(load(&text), ConfigError::Parse(_)), "{bad:?}");
    }
}

#[test]
fn env_values_are_checked() {
    for bad in ["", "a\nb"] {
        let text = format!("[accounts.comp-claude]\nowner = \"comp\"\n[accounts.comp-claude.env]\nA = {bad:?}\n");
        assert!(matches!(load(&text), ConfigError::Parse(_)), "{bad:?}");
    }
    assert_eq!("x y".parse::<EnvValue>().unwrap().as_str(), "x y");
}

/// Every error a configuration or its secrets can give is built from a pair of files whose secrets hold the marker;
/// none of them shows it, in its message or in its debug text.
#[test]
fn secret_marker_not_in_config_errors() {
    let secrets_text = format!(
        "[account_env.comp-claude]\nCLAUDE_CONFIG_DIR = \"{MARKER}\"\nTOKEN = \"{MARKER}\"\n[account_env.other]\nA = \"{MARKER}\"\n"
    );
    let secrets = Secrets::parse(&secrets_text, OWNER_ONLY).unwrap();
    let valid = Config::from_toml_str(&with_account("[accounts.comp-claude.env]\nCLAUDE_CONFIG_DIR = \"/x\"")).unwrap();
    let mut shown = vec![];
    let secrets_error = valid.check_account_env(&secrets).unwrap_err();
    shown.push(format!("{secrets_error} {secrets_error:?}"));
    let other = Config::from_toml_str(&with_account("")).unwrap();
    let only_other = Secrets::parse(&format!("[account_env.other]\nA = \"{MARKER}\"\n"), OWNER_ONLY).unwrap();
    let no_account = other.check_account_env(&only_other).unwrap_err();
    shown.push(format!("{no_account} {no_account:?}"));
    for text in [
        "[accounts.comp-claude]\n[investigators.comp-inv]\ncli = \"claude\"\naccount = \"comp-claude\"\n".to_owned(),
        "[accounts.comp-claude]\nowner = \"\"\n".to_owned(),
        "[accounts.comp-claude]\nowner = \"Comp X\"\n".to_owned(),
        "[accounts.comp-claude]\nowner = \"comp\"\n".to_owned(),
        "[investigators.comp-inv]\ncli = \"claude\"\naccount = \"comp-claude\"\n".to_owned(),
        named_investigator("inv-opus"),
        named_investigator("compx-inv-opus"),
    ] {
        let error = load(&text);
        shown.push(format!("{error} {error:?}"));
    }

    assert_eq!(shown.len(), 9);
    for text in shown {
        assert!(!text.contains(MARKER), "{text}");
    }
}

#[test]
fn an_account_name_of_64_bytes_loads_and_one_of_65_is_refused() {
    let longest = "a".repeat(64);
    let too_long = "a".repeat(65);

    assert_eq!(longest.parse::<AccountName>().unwrap().to_string(), longest);
    let refused = too_long.parse::<AccountName>().unwrap_err();
    assert_eq!(refused.rule, ValueRule::TooLong { max: 64 });
    assert_eq!(refused.value, too_long);
}

#[test]
fn an_owner_of_32_bytes_loads_and_one_of_33_is_refused() {
    let longest = "a".repeat(32);
    let too_long = "a".repeat(33);

    assert_eq!(longest.parse::<OwnerName>().unwrap().to_string(), longest);
    let refused = too_long.parse::<OwnerName>().unwrap_err();
    assert_eq!(refused.rule, ValueRule::TooLong { max: 32 });
    assert_eq!(refused.value, too_long);
}

#[test]
fn an_env_value_of_4096_bytes_loads_and_one_of_4097_is_refused() {
    let longest = "v".repeat(4096);
    let too_long = "v".repeat(4097);

    assert_eq!(longest.parse::<EnvValue>().unwrap().to_string(), longest);
    let refused = too_long.parse::<EnvValue>().unwrap_err();
    assert_eq!(refused.rule, ValueRule::TooLong { max: 4096 });
}

#[test]
fn an_env_value_of_the_longest_size_loads_from_the_file_and_a_longer_one_is_refused_there() {
    let longest = format!("CLAUDE_CONFIG_DIR = \"{}\"", "v".repeat(4096));
    let too_long = format!("CLAUDE_CONFIG_DIR = \"{}\"", "v".repeat(4097));

    assert!(Config::from_toml_str(&with_account(&format!("env = {{ {longest} }}"))).is_ok());
    assert!(matches!(
        Config::from_toml_str(&with_account(&format!("env = {{ {too_long} }}"))).unwrap_err(),
        ConfigError::Parse(_)
    ));
}
