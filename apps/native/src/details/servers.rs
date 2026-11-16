use crate::m;
use hoshi_core::{LocalServer, ServerOrigin};
use std::time::Duration;

/// Rows shown before the rest folds behind "Show more".
pub const SERVERS_SHOWN: usize = 4;

/// How long a server has run, in its largest whole unit, such as "12 min".
pub fn server_age(started_at: u64, now: u64) -> String {
    let seconds = now.saturating_sub(started_at) / 1000;
    match seconds {
        86_400.. => m::server_age_days((seconds / 86_400) as usize),
        3_600.. => m::server_age_hours(seconds / 3_600),
        60.. => m::server_age_minutes(seconds / 60),
        _ => m::server_age_seconds(seconds),
    }
}

/// What started it, its process and its age, leaving out what is unknown.
pub fn server_detail(server: &LocalServer, now: u64) -> String {
    let origin = match server.origin {
        ServerOrigin::Agent => m::server_origin_agent(),
        ServerOrigin::Terminal => m::server_origin_terminal(),
    };
    [
        Some(origin),
        server.process_name.clone(),
        server.started_at.map(|at| server_age(at, now)),
    ]
    .into_iter()
    .flatten()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// How often the servers' ages are told again: each second while one is in its first minute.
pub fn age_tick(servers: &[LocalServer], now: u64) -> Duration {
    let young = servers.iter().any(|server| {
        server
            .started_at
            .is_some_and(|at| now.saturating_sub(at) < 60_000)
    });
    Duration::from_secs(if young { 1 } else { 30 })
}

/// Whether a server is stopped from its row. While the agent runs commands in the background,
/// one of them may be the server: ending it under the agent would have it start the command
/// again, so that one is stopped where the transcript shows it running.
pub fn stops_from_row(server: &LocalServer, commands_running: usize) -> bool {
    server.origin == ServerOrigin::Terminal || commands_running == 0
}

pub fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::{age_tick, server_age, server_detail, stops_from_row};
    use hoshi_core::{LocalServer, ServerOrigin};
    use std::time::Duration;

    const NOW: u64 = 1_794_312_000_000;

    fn server() -> LocalServer {
        LocalServer {
            port: 18400,
            pid: 4242,
            process_name: Some("java".into()),
            origin: ServerOrigin::Agent,
            started_at: Some(NOW - 12 * 60_000 - 30_000),
        }
    }

    #[test]
    fn a_server_is_stopped_from_its_row_unless_a_background_command_may_be_it() {
        assert!(stops_from_row(&server(), 0));
        assert!(!stops_from_row(&server(), 1));
        let terminal = LocalServer {
            origin: ServerOrigin::Terminal,
            ..server()
        };
        assert!(stops_from_row(&terminal, 2));
    }

    #[test]
    fn a_young_server_has_its_age_told_each_second() {
        let young = LocalServer {
            started_at: Some(NOW - 5_000),
            ..server()
        };
        assert_eq!(age_tick(&[young], NOW), Duration::from_secs(1));
        let unknown = LocalServer {
            started_at: None,
            ..server()
        };
        assert_eq!(age_tick(&[server(), unknown], NOW), Duration::from_secs(30));
        assert_eq!(age_tick(&[], NOW), Duration::from_secs(30));
    }

    #[test]
    fn the_age_is_told_in_its_largest_whole_unit() {
        assert_eq!(server_age(NOW - 40_000, NOW), "40 sec");
        assert_eq!(server_age(NOW - 12 * 60_000 - 59_000, NOW), "12 min");
        assert_eq!(server_age(NOW - 2 * 3_600_000 - 1, NOW), "2 hr");
        assert_eq!(server_age(NOW - 86_400_000, NOW), "1 day");
        assert_eq!(server_age(NOW - 3 * 86_400_000, NOW), "3 days");
        assert_eq!(server_age(NOW + 5_000, NOW), "0 sec");
    }

    #[test]
    fn the_detail_leaves_out_what_is_unknown() {
        assert_eq!(server_detail(&server(), NOW), "Agent · java · 12 min");
        let terminal = LocalServer {
            process_name: None,
            origin: ServerOrigin::Terminal,
            ..server()
        };
        assert_eq!(server_detail(&terminal, NOW), "Terminal · 12 min");
        let unknown = LocalServer {
            started_at: None,
            ..server()
        };
        assert_eq!(server_detail(&unknown, NOW), "Agent · java");
    }
}
