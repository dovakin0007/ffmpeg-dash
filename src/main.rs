use std::{
    error::Error, ffi::c_int, ops::{Deref, DerefMut}, path::Path, ptr::null_mut
};

use ffmpeg_next::{
    Codec, Format, Frame, Stream, codec::{Context, Parameters}, decoder::{self, Opened}, ffi::{avformat_close_input, avformat_free_context}, format::{self, Context as FormatContext, Input, Output, context::{input::PacketIter, output::dump}, open}, packet
};

pub struct CodecCtx {
    open: Opened,
}

impl CodecCtx {
    pub fn new(
        codec_params: Parameters,
        thread_count: Option<i32>,
    ) -> Result<CodecCtx, std::string::String> {
        let codec = decoder::find(codec_params.id());
        let codec = codec.ok_or("Unable to find code")?;
        let mut ctx = Context::new_with_codec(codec);
        unsafe {
            (*ctx.as_mut_ptr()).thread_count = thread_count.unwrap_or(1) as c_int;
        }
        let open = ctx
            .decoder()
            .open()
            .map_err(|e| format!("failed to decode: {}", e.to_string()))?;

        return Ok(Self { open });
    }

    pub fn push_packet<P: packet::Ref>(&mut self, packet: &P) -> Result<(), String> {
        self.send_packet(packet)
            .map_err(|e| format!("failed to send packet: {}", e.to_string()))?;
        Ok(())
    }

    pub fn pull_packet(&mut self, frame: &mut Frame) -> Result<(), String> {
        self.receive_frame(frame)
            .map_err(|e| format!("failed to receive packet: {}", e.to_string()))?;
        Ok(())
    }
}

impl Drop for CodecCtx {
    fn drop(&mut self) {
        self.flush();
    }
}

impl Deref for CodecCtx {
    type Target = Opened;

    fn deref(&self) -> &Self::Target {
        &self.open
    }
}

impl DerefMut for CodecCtx {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.open
    }
}

pub struct DemuxerCtx {
    ctx: FormatContext,
}

impl DemuxerCtx {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<DemuxerCtx, String> {
        let av_format_ctx_ptr = null_mut();
        let input = unsafe { Input::wrap(av_format_ctx_ptr) };
        let format = Format::Input(input);

        let ctx = open(&path, &format).map_err(|e| format!("error: {}", e.to_string()))?;

        Ok(Self { ctx: ctx })
    }

}


// impl Drop for DemuxerCtx {
//     fn drop(&mut self) {


//         unsafe {
//             self.ctx {
//                 match ctx {
//                     FormatContext::Input(input) => {
//                         avformat_close_input(input.as_mut_ptr() as *mut *mut _);
//                     }
//                     FormatContext::Output(output) => {
//                         avformat_free_context(output.as_mut_ptr());
//                     }
//                 }
//             }
//         }
//     }
// }


pub struct WrappedFrame {
    pub frame: Frame
}

impl WrappedFrame {
    pub fn new() -> WrappedFrame {
        let frame = unsafe {
            Frame::empty()
        };
        Self { frame }
    }
    pub fn is_video(&self) -> bool {
        unsafe {
            self.as_ptr().read().width > 0
        }
       
    }

}

impl Deref for WrappedFrame {
    type Target = Frame;

    fn deref(&self) -> &Self::Target {
        &self.frame
    }
}

impl DerefMut for WrappedFrame {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.frame
    }
}



fn convert_input_streams(opened: DemuxerCtx, output_ctx: format::Context) -> format::context::Output {
    let input = opened.ctx.input();
    let mut stream_list = Vec::with_capacity(input.nb_streams() as usize);
    let mut output: format::context::Output = output_ctx.output();
    for stream in input.streams().into_iter() {
    
        let in_params = stream.parameters();
 
        match in_params.medium() {
            ffmpeg_next::media::Type::Video | ffmpeg_next::media::Type::Audio | ffmpeg_next::media::Type::Subtitle  => {
            let codec = unsafe {
                Codec::wrap(null_mut())
            };
            // TODO: handle unwrap
            let mut op = output.add_stream(codec).unwrap();
            op.set_parameters(in_params);
            },
            _ => {
                stream_list[stream.index()] = -1
            }
        } 
    }
    dump(&output, 0, None);
    output
    
}

pub fn new_output_path<P: AsRef<Path>>(path: &P) -> Result<format::Context, ffmpeg_next::Error> {
    let output_ctx = null_mut();
    let format = unsafe {Format::Output(Output::wrap(output_ctx))};
    open(path, &format) 
}

fn main() {
    println!("Hello, world!");
}
