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
    pub jsonl_output: Option<String>,

    #[arg(
        short = 'b',
        long = "blanked-subdir",
        default_value_t = false,
        help = "Route files that needed the literal-blanker fallback to a \
                sibling 'jsonl_blanked/' sub-directory"
    )]
    pub blanked_subdir: bool,
}
