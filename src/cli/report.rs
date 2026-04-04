use crate::cli::discovery::discover_speq_root;
use crate::cli::run::{write_allure_from_summary, SummaryReport};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub enum ReportFormat {
    Summary,
    Allure,
    All,
}

#[derive(Debug, Clone)]
pub struct ReportOptions {
    pub speq_root_override: Option<String>,
    pub format: ReportFormat,
    pub summary_input: Option<String>,
}

fn now_ms() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|x| x.as_millis())
        .map_err(|e| format!("internal: failed to read time: {e}"))
}

pub fn build_report_options(
    speq_root_override: Option<String>,
    format: Option<String>,
    summary_input: Option<String>,
) -> Result<ReportOptions, String> {
    let format = match format.as_deref() {
        None => ReportFormat::Allure,
        Some("summary") => ReportFormat::Summary,
        Some("allure") => ReportFormat::Allure,
        Some("all") => ReportFormat::All,
        Some(other) => {
            return Err(format!(
                "unsupported report format '{}', expected all|summary|allure",
                other
            ))
        }
    };
    Ok(ReportOptions {
        speq_root_override,
        format,
        summary_input,
    })
}

pub fn command_report(options: ReportOptions) -> Result<(), String> {
    let discovered = discover_speq_root(options.speq_root_override)?;
    let reports_root = discovered.root.join("reports");
    let summary_path = if let Some(raw) = options.summary_input {
        let p = PathBuf::from(raw);
        if p.is_absolute() {
            p
        } else {
            discovered.root.join(p)
        }
    } else {
        reports_root.join("results").join("summary.json")
    };
    let allure_dir = reports_root.join("allure");

    if !summary_path.is_file() {
        return Err(format!(
            "summary input not found: {} (run `speq run --report summary|all` first)",
            summary_path.display()
        ));
    }

    let content = fs::read_to_string(&summary_path)
        .map_err(|e| format!("failed to read summary {}: {e}", summary_path.display()))?;
    let summary = serde_json::from_str::<SummaryReport>(&content)
        .map_err(|e| format!("invalid summary json {}: {e}", summary_path.display()))?;

    match options.format {
        ReportFormat::Summary => {}
        ReportFormat::Allure => {
            write_allure_from_summary(&allure_dir, &format!("report-{}", now_ms()?), &summary)?;
        }
        ReportFormat::All => {
            write_allure_from_summary(&allure_dir, &format!("report-{}", now_ms()?), &summary)?;
        }
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
          "ok": true,
          "summary": summary_path.to_string_lossy(),
          "allure": match options.format {
            ReportFormat::Summary => None::<String>,
            _ => Some(allure_dir.to_string_lossy().to_string())
          }
        }))
        .map_err(|e| format!("internal: failed to encode report response: {e}"))?
    );

    Ok(())
}
