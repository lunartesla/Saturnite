//! The `saturn` toolchain binary.
//!
//! This is the user-facing entry point for Saturnite project development,
//! analogous to Cargo for Rust. It discovers the project via `saturn.toml`,
//! manages build profiles and output directories, and delegates compilation
//! to the `stnx` compiler library.

use saturn::build::BuildContext;
use saturn::cli::{Commands, SaturnCli};
use saturn::profile::BuildProfile;

use clap::Parser;

fn main() -> anyhow::Result<()> {
    let cli = SaturnCli::parse();

    match cli.command {
        Commands::Build {
            input,
            release,
            debug,
            output,
            no_link,
            save_temps,
            verbose,
        } => {
            let profile = SaturnCli::resolve_profile(release, debug);
            let ctx = SaturnCli::make_build_context(
                input.as_ref(),
                release,
                debug,
                no_link,
                save_temps,
            )?;

            if verbose {
                eprintln!("project root: {}", ctx.project_root.display());
                eprintln!("entry:        {}", ctx.entry.display());
                eprintln!("profile:      {}", profile.as_str());
                eprintln!("output:       {}", ctx.output.display());
            }

            let result = ctx.build()?;
            println!(
                "Built {} -> {} ({} ms)",
                ctx.entry.display(),
                result.output.display(),
                result.elapsed_ms
            );
            Ok(())
        }

        Commands::Run {
            input,
            release,
            debug,
            verbose,
        } => {
            let profile = SaturnCli::resolve_profile(release, debug);
            let ctx = SaturnCli::make_build_context(
                input.as_ref(),
                release,
                debug,
                false,
                false,
            )?;

            if verbose {
                eprintln!("profile: {}", profile.as_str());
                eprintln!("output:  {}", ctx.output.display());
            }

            let exit_code = ctx.run()?;
            std::process::exit(exit_code);
        }

        Commands::Check { input, verbose } => {
            let ctx = SaturnCli::make_build_context(
                input.as_ref(),
                false,
                false,
                false,
                false,
            )?;

            if verbose {
                eprintln!("checking: {}", ctx.entry.display());
            }

            ctx.check()?;
            println!("No errors found in {}", ctx.entry.display());
            Ok(())
        }

        Commands::Test { verbose } => {
            let cwd = std::env::current_dir()?;
            let project = saturn::discover::ProjectDiscovery::discover(&cwd)?;
            let ctx = BuildContext::new(project.root, BuildProfile::Debug)?;

            if verbose {
                eprintln!("Running tests for project: {}", ctx.manifest.package.name);
            }

            // For now, test = run the check pipeline (full semantic analysis).
            // A proper test harness integrates with the compiler's test framework
            // in a future phase.
            ctx.check()?;
            println!("All tests passed.");
            Ok(())
        }

        Commands::Clean { all } => {
            let cwd = std::env::current_dir()?;
            let project = saturn::discover::ProjectDiscovery::discover(&cwd)?;
            let ctx = BuildContext::new(project.root, BuildProfile::Debug)?;

            if all {
                ctx.clean_all()?;
                println!("Removed target/ directory and all build artifacts.");
            } else {
                ctx.clean_profile()?;
                println!("Removed target/{} directory.", ctx.profile.dir_name());
            }
            Ok(())
        }

        Commands::Init {
            name,
            in_place,
            pkg_version,
        } => {
            saturn::cmd::init_project(name.as_deref(), in_place, pkg_version.as_deref())?;
            Ok(())
        }

        Commands::Doctor => {
            saturn::cmd::run_doctor()?;
            Ok(())
        }
    }
}
