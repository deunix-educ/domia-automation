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
    /// Demarre le serveur (HTTP, puis MQTT, planificateur, video... selon les roles).
    Serve(ServeArgs),
    /// Applique les migrations de la base de donnees.
    Migrate,
    /// Cree les donnees initiales (reglages, navigation) sans jamais ecraser l'existant.
    Seed(SeedArgs),
    /// Cree les libelles manquants (nom, icone, unite) des cles d'etat presentes sur les appareils.
    SeedLabels(SeedArgs),
    /// Cree la source MQTT « Meteo », l'appareil `meteo_locale` et (avec --lat et --lon) une station Open-Meteo.
    SeedWeather(SeedWeatherArgs),
    /// Ajoute aux appareils du bilan d'energie les cles d'etat a suivre (compteurs Linky, PAC, `qdy` du SolarMax).
    SeedEnergy(SeedArgs),
    /// Calcule (ou recalcule) le bilan d'energie des derniers jours a partir de l'historique.
    AggregateEnergy(AggregateEnergyArgs),
    /// Diagnostic d'un publisher : un seul cycle (lecture de la source, affichage, publication sur le broker).
    Publish(PublishArgs),
    /// Exporte un instantane YAML de l'installation (appareils et leurs etats, schemas de reference).
    DumpContext(DumpContextArgs),
    /// Envoie un vrai message de test par chaque canal demande (e-mail, SMS Free Mobile). A lancer soi-meme : rien d'autre n'envoie.
    NotifyTest(NotifyTestArgs),
    /// Sauvegarde maintenant la base (`pg_dump`) et les donnees `ReductStore` dans `BACKUP_DIR`, puis purge les sauvegardes expirees.
    Backup,
    /// Liste les sauvegardes presentes dans `BACKUP_DIR`.
    Backups,
    /// Restaure une sauvegarde : la base (fichier `.sql.gz`, DETRUIT les donnees actuelles : exige `--yes`) ou le volume `ReductStore` (fichier `.tar.gz` vers `--to`).
    Restore(RestoreArgs),
    /// Ecrit les fichiers de regles d'alerte (Prometheus, Grafana) depuis la base et demande aux services de les relire.
    SyncRules,
    /// Sonde `/healthz` d'un serveur local (pour le healthcheck de Docker, l'image n'a pas `curl`) : code 0 si 200, sinon 1.
    Healthcheck(HealthcheckArgs),
    /// Diagnostic en lecture seule vu depuis l'application (base, migrations, administrateur, cles, MQTT, ReductStore) ; code de sortie 1 s'il y a un point bloquant.
    Doctor,
    /// Affiche une nouvelle cle maitresse (a copier dans `DOMINET_MASTER_KEY`).
    GenMasterKey,
    /// Cree un compte administrateur.
    Createsuperuser(SuperuserArgs),
    /// Change le mot de passe d'un compte.
    Passwd(UserArgs),
}

#[derive(Debug, clap::Args)]
pub struct HealthcheckArgs {
    /// Adresse a sonder (defaut : le port de `HTTP_ADDR`, sur 127.0.0.1).
    #[arg(long)]
    pub addr: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct RestoreArgs {
    /// Fichier de sauvegarde (`.sql.gz` pour la base, `.tar.gz` pour ReductStore).
    pub file: Option<std::path::PathBuf>,
    /// A la place d'un fichier : la plus recente sauvegarde de cette nature.
    #[arg(long, value_parser = ["postgres", "reductstore"], conflicts_with = "file")]
    pub latest: Option<String>,
    /// Dossier de donnees ou extraire l'archive ReductStore (ReductStore arrete).
    #[arg(long)]
    pub to: Option<std::path::PathBuf>,
    /// Confirme la restauration de la base (elle remplace les donnees actuelles).
    #[arg(long)]
    pub yes: bool,
    /// Extrait l'archive ReductStore meme si le dossier cible n'est pas vide.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, clap::Args)]
pub struct NotifyTestArgs {
    /// Envoie un e-mail de test (au destinataire des alertes, ou a `--to`).
    #[arg(long)]
    pub email: bool,
    /// Envoie un SMS de test par Free Mobile.
    #[arg(long)]
    pub sms: bool,
    /// Destinataire de l'e-mail de test (defaut : `EMAIL_RECIPIENT`).
    #[arg(long)]
    pub to: Option<String>,
}

#[derive(Debug, clap::Args)]
pub struct ServeArgs {
    /// Ne lancer que ces roles (defaut : tous les roles disponibles).
    #[arg(long, value_enum, value_delimiter = ',')]
    pub only: Vec<Role>,
}

#[derive(Debug, clap::Args)]
pub struct SeedArgs {
    /// Montre ce qui serait cree, sans rien ecrire.
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Debug, clap::Args)]
pub struct AggregateEnergyArgs {
    /// Nombre de jours passes a calculer (defaut : hier seulement).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=3650))]
    pub days: u32,
    /// Un jour precis (AAAA-MM-JJ, jour local) ; remplace --days.
    #[arg(long)]
    pub date: Option<chrono::NaiveDate>,
}

#[derive(Debug, clap::Args)]
pub struct SeedWeatherArgs {
    /// Latitude de la station (avec --lon : cree la station).
    #[arg(long, allow_hyphen_values = true)]
    pub lat: Option<f64>,
    /// Longitude de la station.
    #[arg(long, allow_hyphen_values = true)]
    pub lon: Option<f64>,
    /// Altitude en metres.
    #[arg(long, default_value_t = 0, allow_hyphen_values = true)]
    pub alt: i32,
    /// Fuseau horaire transmis a Open-Meteo.
    #[arg(long, default_value = "Europe/Paris")]
    pub timezone: String,
    /// Intervalle d'interrogation, en minutes.
    #[arg(long, default_value_t = 10)]
    pub refresh: i16,
    /// Montre ce qui serait cree, sans rien ecrire.
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
    /// Source a lire.
    #[arg(value_enum)]
    pub target: PublishTarget,
    /// Affiche le resultat sans rien publier sur le broker.
    #[arg(long)]
    pub print_only: bool,
}

#[derive(Debug, clap::Args)]
pub struct DumpContextArgs {
    /// Ecrit le document dans ce fichier (par defaut : sur la sortie standard).
    #[arg(long)]
    pub output: Option<std::path::PathBuf>,
}

#[derive(Debug, clap::Args)]
pub struct SuperuserArgs {
    /// Nom d'utilisateur (demande en interactif s'il est omis).
    #[arg(long)]
    pub username: Option<String>,
    /// Adresse e-mail (facultative).
    #[arg(long, default_value = "")]
    pub email: String,
}

#[derive(Debug, clap::Args)]
pub struct UserArgs {
    /// Nom d'utilisateur (demande en interactif s'il est omis).
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
    /// Premier jalon de la feuille de route ou le role existe.
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
