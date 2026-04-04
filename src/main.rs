use std::env;
use speq_cli::cli;

const EXIT_OK: i32 = 0;
const EXIT_VALIDATION_CONFIG: i32 = 2;
const EXIT_INTERNAL: i32 = 3;

fn usage() {
    println!(
        "speq CLI\n\
         \n\
         Commands:\n\
          speq init [--mode in-repo|test-repo]\n\
          speq list [--speq-root <path>] [--format json]\n\
          speq run [--speq-root <path>] [--env <name>] [--test <file>|--suite <dir>] [--tags a,b] [--report all|summary|allure] [--output <summary.json>]\n\
          speq report [--speq-root <path>] [--format all|summary|allure] [--summary <summary.json>]\n\
          speq doctor [--speq-root <path>] [--format json]\n\
           speq validate [--speq-root <path>] [--format json]\n\
           speq help\n"
    );
}

fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == flag)
        .map(|w| w[1].clone())
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|a| a == flag)
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
        std::process::exit(EXIT_VALIDATION_CONFIG);
    }

    let command = args[1].as_str();
    let command_args = &args[2..];

    let result = match command {
        "init" => {
            let mode = parse_flag_value(command_args, "--mode");
            cli::init::command_init(mode).map(|_| EXIT_OK)
        }
        "list" => {
            let speq_root = parse_flag_value(command_args, "--speq-root");
            let format_json = has_flag(command_args, "--format")
                && parse_flag_value(command_args, "--format").as_deref() == Some("json");
            cli::list::command_list(speq_root, format_json).map(|_| EXIT_OK)
        }
        "validate" => {
            let speq_root = parse_flag_value(command_args, "--speq-root");
            let format_json = has_flag(command_args, "--format")
                && parse_flag_value(command_args, "--format").as_deref() == Some("json");
            cli::validate::command_validate(speq_root, format_json).map(|_| EXIT_OK)
        }
        "run" => {
            let options = cli::run::build_run_options(
                parse_flag_value(command_args, "--speq-root"),
                parse_flag_value(command_args, "--env"),
                parse_flag_value(command_args, "--test"),
                parse_flag_value(command_args, "--suite"),
                parse_flag_value(command_args, "--tags"),
                parse_flag_value(command_args, "--report"),
                parse_flag_value(command_args, "--output"),
            );
            match options {
                Ok(opts) => cli::run::command_run(opts).await,
                Err(e) => Err(e),
            }
        }
        "report" => {
            let options = cli::report::build_report_options(
                parse_flag_value(command_args, "--speq-root"),
                parse_flag_value(command_args, "--format"),
                parse_flag_value(command_args, "--summary"),
            );
            match options {
                Ok(opts) => cli::report::command_report(opts).map(|_| EXIT_OK),
                Err(e) => Err(e),
            }
        }
        "doctor" => {
            let speq_root = parse_flag_value(command_args, "--speq-root");
            let format_json = has_flag(command_args, "--format")
                && parse_flag_value(command_args, "--format").as_deref() == Some("json");
            cli::doctor::command_doctor(speq_root, format_json).map(|_| EXIT_OK)
        }
        "help" | "-h" | "--help" => {
            usage();
            Ok(EXIT_OK)
        }
        other => Err(format!("unknown command: {other}")),
    };

    match result {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("{err}");
            if err.starts_with("internal:") {
                std::process::exit(EXIT_INTERNAL);
            }
            std::process::exit(EXIT_VALIDATION_CONFIG);
        }
    }
}
