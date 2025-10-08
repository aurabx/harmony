use clap::{Parser, Subcommand};
use dicom_json_tool as tool;
use dicom_object::mem::InMemDicomObject;
use dicom_object::open_file;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "dicom-json",
    about = "Convert between DICOM datasets and DICOM JSON"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Convert a DICOM file to DICOM JSON
    ToJson {
        #[arg(short, long)]
        input: PathBuf,
    },
    /// Convert a DICOM JSON file to a DICOM file
    FromJson {
        #[arg(short, long)]
        input: PathBuf,
        #[arg(short, long)]
        output: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::ToJson { input } => {
            let obj = open_file(&input)?;
            let json = tool::identifier_to_json_value(&obj);
            match json {
                Ok(v) => {
                    println!("{}", serde_json::to_string_pretty(&v)?);
                    Ok(())
                }
                Err(e) => Err(anyhow::anyhow!(e)),
            }
        }
        Cmd::FromJson { input, output } => {
            let text = std::fs::read_to_string(&input)?;
            let v: serde_json::Value = serde_json::from_str(&text)?;
            // Accept either wrapper or raw identifier JSON
            let (_cmd, identifier, _qmeta) = tool::parse_wrapper_or_identifier(&v);
            let obj: InMemDicomObject =
                tool::json_value_to_identifier(&identifier).map_err(|e| anyhow::anyhow!(e))?;
            tool::write_part10(&output, &obj).map_err(|e| anyhow::anyhow!(e))?;
            eprintln!("Wrote Part 10 file to {}", output.display());
            Ok(())
        }
    }
}
