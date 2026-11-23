// SPDX-License-Identifier: AGPL-3.0-or-later
//! `dominet` command line.

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "dominet",
    version,
    about = "DomiNet: self-hosted home automation",
    arg_required_else_help = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Starts the server (HTTP, then MQTT, scheduler, video... depending on the roles).
    Serve(ServeArgs),
    /// Applies the database migrations.
    Migrate,
    /// Creates the initial data (settings, navigation) without ever overwriting what exists.
    Seed(SeedArgs),
    /// Creates the missing labels (name, icon, unit) of the state keys present on the devices.
    SeedLabels(SeedArgs),
    /// Creates the "Meteo" MQTT source, the `meteo_locale` device and (with --lat and --lon) an Open-Meteo station.
    SeedWeather(SeedWeatherArgs),
    /// Adds to the energy summary's devices the state keys to track (Linky meters, heat pump, SolarMax's `qdy`).
    SeedEnergy(SeedArgs),
    /// Computes (or recomputes) the energy summary of the last days from the history.
    AggregateEnergy(AggregateEnergyArgs),
    /// Diagnostic of a publisher: a single cycle (reading the source, display, publishing on the broker).
    Publish(PublishArgs),
    /// Exports a YAML snapshot of the installation (devices and their states, reference schemas).
    DumpContext(DumpContextArgs),
    /// Sends a real test message through each requested channel (e-mail, Free Mobile SMS). To be run by hand: nothing else sends.
    NotifyTest(NotifyTestArgs),
    /// Backs up now the database (`pg_dump`) and the `ReductStore` data into `BACKUP_DIR`, then purges the expired backups.
    Backup,
    /// Lists the backups present in `BACKUP_DIR`.
    Backups,
    /// Restores a backup: the database (`.sql.gz` file, DESTROYS the current data: requires `--yes`) or the `ReductStore` volume (`.tar.gz` file to `--to`).
    Restore(RestoreArgs),
    /// Writes the alert-rule files (Prometheus, Grafana) from the database and asks the services to re-read them.
    SyncRules,
    /// Probes `/healthz` of a local server (for Docker's healthcheck, the image has no `curl`): exit code 0 if 200, otherwise 1.
    Healthcheck(HealthcheckArgs),
    /// Read-only diagnostic seen from the application (database, migrations, administrator, keys, MQTT, ReductStore); exit code 1 if there is a blocking point.
    Doctor,
    /// Prints a new master key (to copy into `DOMINET_MASTER_KEY`).
    GenMasterKey,
    /// Creates an administrator account.
    Createsuperuser(SuperuserArgs),
    /// Changes an account's password.
    Passwd(UserArgs),
}

#[derive(Debug, clap::Args)]
pub struct HealthcheckArgs {
    /// Address to probe (default: the port of `HTTP_ADDR`, on 127.0.0.1).
    #[arg(long)]
    pub addr: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct RestoreArgs {
    /// Backup file (`.sql.gz` for the database, `.tar.gz` for ReductStore).
    pub file: Option<std::path::PathBuf>,
    /// Instead of a file: the most recent backup of that kind.
    #[arg(long, value_parser = ["postgres", "reductstore"], conflicts_with = "file")]
    pub latest: Option<String>,
    /// Data folder to extract the ReductStore archive into (ReductStore stopped).
    #[arg(long)]
    pub to: Option<std::path::PathBuf>,
    /// Confirms the database restore (it replaces the current data).
    #[arg(long)]
    pub yes: bool,
    /// Extracts the ReductStore archive even if the target folder is not empty.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct NotifyTestArgs {
    /// Sends a test e-mail (to the alerts recipient, or to `--to`).
    #[arg(long)]
    pub email: bool,
    /// Sends a test SMS through Free Mobile.
    #[arg(long)]
    pub sms: bool,
    /// Recipient of the test e-mail (default: `EMAIL_RECIPIENT`).
    #[arg(long)]
    pub to: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ServeArgs {
    /// Only run these roles (default: all the available roles).
    #[arg(long, value_enum, value_delimiter = ',')]
    pub only: Vec<Role>,
}

#[derive(Debug, clap::Args)]
pub struct SeedArgs {
    /// Shows what would be created, without writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, clap::Args)]
pub struct AggregateEnergyArgs {
    /// Number of past days to compute (default: yesterday only).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=3650))]
    pub days: u32,
    /// A specific day (YYYY-MM-DD, local day); replaces --days.
    #[arg(long)]
    pub date: Option<chrono::NaiveDate>,
}

#[derive(Debug, clap::Args)]
pub struct SeedWeatherArgs {
    /// Latitude of the station (with --lon: creates the station).
    #[arg(long, allow_hyphen_values = true)]
    pub lat: Option<f64>,
    /// Longitude of the station.
    #[arg(long, allow_hyphen_values = true)]
    pub lon: Option<f64>,
    /// Altitude in metres.
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    pub alt: i32,
    /// Time zone passed to Open-Meteo.
    #[arg(long, default_value = "Europe/Paris")]
    pub timezone: String,
    /// Polling interval, in minutes.
    #[arg(long, default_value_t = 10)]
    pub refresh: i16,
    /// Shows what would be created, without writing anything.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PublishTarget {
    Weather,
    Solarmax,
}

#[derive(Debug, clap::Args)]
pub struct PublishArgs {
    /// Source to read.
    #[arg(value_enum)]
    pub target: PublishTarget,
    /// Prints the result without publishing anything on the broker.
    #[arg(long)]
    pub print_only: bool,
}

#[derive(Debug, clap::Args)]
pub struct DumpContextArgs {
    /// Writes the document to this file (default: on standard output).
    #[arg(long)]
    pub output: Option<std::path::PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct SuperuserArgs {
    /// Username (asked interactively if omitted).
    #[arg(long)]
    pub username: Option<String>,
    /// E-mail address (optional).
    #[arg(long, default_value = "")]
    pub email: String,
}

#[derive(Debug, clap::Args)]
pub struct UserArgs {
    /// Username (asked interactively if omitted).
    #[arg(long)]
    pub username: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Role {
    Web,
    Mqtt,
    Scheduler,
    Video,
    Publishers,
    Discovery,
}

impl Role {
    /// First roadmap milestone where the role exists.
    #[must_use]
    pub fn available_since(self) -> Option<&'static str> {
        match self {
            Self::Web
            | Self::Mqtt
            | Self::Publishers
            | Self::Scheduler
            | Self::Discovery
            | Self::Video => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn serve_only_accepts_a_list_of_roles() {
        let cli = Cli::try_parse_from(["dominet", "serve", "--only", "web,mqtt"]).unwrap();
        let Command::Serve(args) = cli.command else {
            panic!("serve attendu")
        };
        assert_eq!(args.only, [Role::Web, Role::Mqtt]);
    }

    #[test]
    fn unknown_role_is_rejected() {
        assert!(Cli::try_parse_from(["dominet", "serve", "--only", "cloud"]).is_err());
    }

    #[test]
    fn account_commands_take_a_username() {
        let cli = Cli::try_parse_from([
            "dominet",
            "createsuperuser",
            "--username",
            "admin",
            "--email",
            "a@b.c",
        ])
        .unwrap();
        let Command::Createsuperuser(a) = cli.command else {
            panic!("createsuperuser attendu")
        };
        assert_eq!(
            (a.username.as_deref(), a.email.as_str()),
            (Some("admin"), "a@b.c")
        );
        let cli = Cli::try_parse_from(["dominet", "passwd", "--username", "bob"]).unwrap();
        let Command::Passwd(a) = cli.command else {
            panic!("passwd attendu")
        };
        assert_eq!(a.username.as_deref(), Some("bob"));
    }

    #[test]
    fn seed_and_master_key_commands() {
        let cli = Cli::try_parse_from(["dominet", "seed", "--dry-run"]).unwrap();
        let Command::Seed(a) = cli.command else {
            panic!("seed attendu")
        };
        assert!(a.dry_run);
        let cli = Cli::try_parse_from(["dominet", "seed"]).unwrap();
        let Command::Seed(a) = cli.command else {
            panic!("seed attendu")
        };
        assert!(!a.dry_run);
        let cli = Cli::try_parse_from(["dominet", "seed-labels", "--dry-run"]).unwrap();
        assert!(matches!(cli.command, Command::SeedLabels(a) if a.dry_run));
        assert!(matches!(
            Cli::try_parse_from(["dominet", "gen-master-key"])
                .unwrap()
                .command,
            Command::GenMasterKey
        ));
        assert!(matches!(
            Cli::try_parse_from(["dominet", "doctor"]).unwrap().command,
            Command::Doctor
        ));
    }

    #[test]
    fn missing_subcommand_is_an_error() {
        assert!(Cli::try_parse_from(["dominet"]).is_err());
    }

    #[test]
    fn version_flag_is_available() {
        let err = Cli::try_parse_from(["dominet", "--version"]).unwrap_err();
        assert_eq!(err.kind(), clap::error::ErrorKind::DisplayVersion);
    }

    #[test]
    fn the_lot_6_commands_parse() {
        let cli = Cli::try_parse_from(["dominet", "publish", "solarmax", "--print-only"]).unwrap();
        assert!(
            matches!(cli.command, Command::Publish(a) if a.print_only && a.target == PublishTarget::Solarmax)
        );
        let cli = Cli::try_parse_from(["dominet", "publish", "weather"]).unwrap();
        assert!(
            matches!(cli.command, Command::Publish(a) if !a.print_only && a.target == PublishTarget::Weather)
        );
        assert!(Cli::try_parse_from(["dominet", "publish", "inconnu"]).is_err());
        let cli = Cli::try_parse_from([
            "dominet",
            "seed-weather",
            "--lat",
            "-43.3",
            "--lon",
            "5.4",
            "--refresh",
            "15",
        ])
        .unwrap();
        let Command::SeedWeather(a) = cli.command else {
            panic!()
        };
        assert_eq!(
            (a.lat, a.lon, a.alt, a.refresh, a.timezone.as_str()),
            (Some(-43.3), Some(5.4), 0, 15, "Europe/Paris")
        );
        let cli = Cli::try_parse_from(["dominet", "dump-context", "--output", "ctx.yaml"]).unwrap();
        assert!(matches!(cli.command, Command::DumpContext(a) if a.output.is_some()));
        let cli = Cli::try_parse_from(["dominet", "serve", "--only", "mqtt,publishers"]).unwrap();
        let Command::Serve(a) = cli.command else {
            panic!()
        };
        assert_eq!(a.only, [Role::Mqtt, Role::Publishers]);
        assert_eq!(Role::Publishers.available_since(), None);
    }
}
