use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(
        short,
        long,
        help = "Input directory of the set of java files that needs to be converted"
    )]
    pub input: String,

    #[arg(
        short,
        long,
        help = "Output directory of the set of converted java files"
    )]
    pub output: String,

    #[arg(short, long, help = "Output directory for the jsonL files")]
    pub jsonl_output: String,
}
