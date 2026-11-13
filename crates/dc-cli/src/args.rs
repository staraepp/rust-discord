//! Command-line parsing. Deliberately tiny: no token-bearing flags exist.

use dc_protocol::Snowflake;

pub const USAGE: &str = "\
dc-cli - headless dev tool for the native Discord client (M0 protocol spike)

USAGE:
    dc-cli guilds                      Connect, list guilds and channels, exit
    dc-cli watch <channel_id>          Print new messages in one channel (Ctrl-C to stop)
    dc-cli history <channel_id> [N]    Print the last N messages over REST (1-100, default 20)

ENVIRONMENT:
    DC_TOKEN    Account token. Read from the environment only; it is never
                accepted as an argument or read from a file.
    DC_LOG      Log level on stderr: error, warn (default), info, debug, trace
";

pub const DEFAULT_HISTORY: u8 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Guilds,
    Watch { channel_id: Snowflake },
    History { channel_id: Snowflake, limit: u8 },
    Help,
}

pub fn parse<I, S>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let args: Vec<S> = args.into_iter().collect();
    let args: Vec<&str> = args.iter().map(AsRef::as_ref).collect();

    // Error messages below never echo argument values: a mistyped command
    // line could contain the token.
    if args
        .iter()
        .any(|a| a.to_ascii_lowercase().contains("token"))
    {
        return Err(
            "the token is only read from the DC_TOKEN environment variable, never from arguments"
                .to_owned(),
        );
    }

    match args.as_slice() {
        [] | ["help" | "-h" | "--help"] => Ok(Command::Help),
        ["guilds"] => Ok(Command::Guilds),
        ["watch", channel] => Ok(Command::Watch {
            channel_id: channel_id(channel)?,
        }),
        ["history", channel] => Ok(Command::History {
            channel_id: channel_id(channel)?,
            limit: DEFAULT_HISTORY,
        }),
        ["history", channel, limit] => {
            let limit = limit
                .parse::<u8>()
                .ok()
                .filter(|n| (1..=100).contains(n))
                .ok_or("the message count must be a number between 1 and 100")?;
            Ok(Command::History {
                channel_id: channel_id(channel)?,
                limit,
            })
        }
        _ => Err("unrecognized arguments".to_owned()),
    }
}

fn channel_id(text: &str) -> Result<Snowflake, String> {
    text.parse()
        .map_err(|_| "the channel ID must be a number".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands() {
        assert_eq!(parse(Vec::<&str>::new()), Ok(Command::Help));
        assert_eq!(parse(["--help"]), Ok(Command::Help));
        assert_eq!(parse(["guilds"]), Ok(Command::Guilds));
        assert_eq!(
            parse(["watch", "300000000000000002"]),
            Ok(Command::Watch {
                channel_id: Snowflake(300_000_000_000_000_002)
            })
        );
        assert_eq!(
            parse(["history", "42"]),
            Ok(Command::History {
                channel_id: Snowflake(42),
                limit: DEFAULT_HISTORY
            })
        );
        assert_eq!(
            parse(["history", "42", "100"]),
            Ok(Command::History {
                channel_id: Snowflake(42),
                limit: 100
            })
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert!(parse(["watch"]).is_err());
        assert!(parse(["watch", "general"]).is_err());
        assert!(parse(["history", "42", "0"]).is_err());
        assert!(parse(["history", "42", "101"]).is_err());
        assert!(parse(["frobnicate"]).is_err());
        // A token pasted where the channel ID goes must not be echoed.
        let err = parse(["watch", "fake-secret-value"]).unwrap_err();
        assert!(!err.contains("fake-secret-value"), "{err}");
    }

    #[test]
    fn token_arguments_are_refused() {
        for args in [
            vec!["--token", "fake-token-for-tests-only", "guilds"],
            vec!["guilds", "--token=fake-token-for-tests-only"],
            vec!["watch", "42", "-TOKEN"],
        ] {
            let err = parse(args).unwrap_err();
            assert!(err.contains("DC_TOKEN"), "{err}");
            assert!(!err.contains("fake-token"), "echoed the argument: {err}");
        }
    }
}
