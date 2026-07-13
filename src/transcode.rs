use std::path::PathBuf;

use ffmpeg_next::codec::packet;

pub struct TranscoderConfig<T: AsRef<str>> {
    pub input: PathBuf,
    pub output: PathBuf,
    pub encoder_name: Option<T>,
    pub width: u32,
    pub height: u32,
}

impl<T: std::convert::AsRef<str>> TranscoderConfig<T> {
    pub fn new(input: PathBuf, output: PathBuf, encoder_name: Option<T>, width: u32, height: u32) -> TranscoderConfig<T> {
        TranscoderConfig {
            input,
            output,
            encoder_name,
            width,
            height,
        }
    }
}




pub fn run<T: AsRef<str>>(t: TranscoderConfig<T>) ->  anyhow::Result<()> {
    let packet = packet::Packet::empty();
    // let 
    todo!()
}