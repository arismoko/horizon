//! NBT path parser output and byte cursor for the pinned Java oracle.
use std::io::{BufRead, Write};
use sunrise_game_data::nbt_path::NbtPath;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = std::io::BufReader::new(std::fs::File::open(args.next().unwrap())?);
    let mut output = std::io::BufWriter::new(std::fs::File::create(args.next().unwrap())?);
    for line in input.lines() {
        let text: String = serde_json::from_str(&line?)?;
        match NbtPath::parse_prefix(&text) {
            Ok((path, cursor)) => {
                write!(output, "OK {cursor} ")?;
                for value in path.as_str().as_bytes() {
                    write!(output, "{value:02x}")?;
                }
                writeln!(output)?;
            }
            Err(error) => writeln!(output, "ERR {} {}", error.cursor, error.key)?,
        }
    }
    Ok(())
}
