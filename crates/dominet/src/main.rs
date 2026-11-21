// SPDX-License-Identifier: AGPL-3.0-or-later
//! Point d'entree du binaire `dominet` : CLI, puis serveur HTTP avec l'interface Leptos.

use std::process::ExitCode;

use anyhow::bail;
use clap::Parser;
use dominet_server::cli::{Cli, Command, Role};
use dominet_server::config::Config;
use dominet_server::{commands, server, telemetry};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    // Cette commande n'a besoin ni de base de donnees ni de configuration.
    if matches!(cli.command, Command::GenMasterKey) {
        commands::gen_master_key();
        return ExitCode::SUCCESS;
    }
    // Idem : la sonde de sante ne lit que HTTP_ADDR (le healthcheck de Docker ne doit pas dependre de la base).
    if let Command::Healthcheck(args) = &cli.command {
        return if commands::healthcheck(args.addr.as_deref()) {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    // La configuration d'abord : une erreur doit etre lisible meme sans journal.
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    telemetry::init(config.log_format);
    match run(cli, config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!(error = ?e, "failure");
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli, config: Config) -> anyhow::Result<()> {
    match cli.command {
        Command::Serve(args) => {
            // Sans `--only` : le web et le planificateur, plus le client MQTT si un broker est configure.
            let roles = if args.only.is_empty() {
                let mut r = vec![Role::Web, Role::Scheduler];
                // Le role video decode des flux : il demarre tout seul quand on lui dit ou est le re-stream de MediaMTX.
                if config.mediamtx_rtsp_url.is_some() {
                    r.push(Role::Video);
                }
                if config.mqtt.is_some() {
                    r.push(Role::Mqtt);
                    r.push(Role::Publishers);
                }
                r
            } else {
                args.only
            };
            if let Some(role) = roles.iter().find(|r| r.available_since().is_some()) {
                bail!(
                    "the {role:?} role does not exist yet (planned for {})",
                    role.available_since().unwrap_or("?")
                );
            }
            let site_root = config.site_root.clone();
            let addr = config.http_addr;
            server::serve(
                config,
                &roles,
                move |state| dominet_web::ssr::ui_service(state.clone(), &site_root, addr),
                dominet_mcp::routes,
            )
            .await
        }
        Command::Doctor => {
            if dominet_server::doctor::run(&config).await > 0 {
                bail!("blocking problems found (see above)");
            }
            Ok(())
        }
        Command::Migrate => commands::migrate(&config).await,
        Command::Seed(args) => commands::seed(&config, args).await,
        Command::SeedLabels(args) => commands::seed_labels(&config, args).await,
        Command::SeedEnergy(args) => commands::seed_energy(&config, args).await,
        Command::AggregateEnergy(args) => commands::aggregate_energy(&config, args).await,
        Command::SeedWeather(args) => commands::seed_weather(&config, args).await,
        Command::Publish(args) => commands::publish(&config, args).await,
        Command::DumpContext(args) => commands::dump_context(&config, args).await,
        Command::NotifyTest(args) => commands::notify_test(&config, args).await,
        Command::SyncRules => commands::sync_rules(&config).await,
        Command::Backup => commands::backup(&config).await,
        Command::Backups => commands::backups(&config),
        Command::Restore(args) => commands::restore(&config, args),
        Command::GenMasterKey | Command::Healthcheck(_) => Ok(()),
        Command::Createsuperuser(args) => commands::createsuperuser(&config, args).await,
        Command::Passwd(args) => commands::passwd(&config, args).await,
    }
}
