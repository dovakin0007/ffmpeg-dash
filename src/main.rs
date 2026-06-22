use std::{
    error::Error, ffi::c_int, ops::{Deref, DerefMut}, path::Path, ptr::null_mut
};

use ffmpeg_next::{
    Codec, Format, Frame, Packet, Rounding, Stream, codec::{Context, Parameters}, decoder::{self, Opened}, device::input, format::{self, Context as FormatContext, Flags, Input, Output, context::output::dump, open}, packet
};
use ffmpeg_next::util::mathematics::rescale::Rescale;

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



fn convert_input_streams(opened: &mut format::context::Input, output_ctx: &mut format::context::Output) -> Vec<i32> {
    let input = opened;
    let mut stream_list = vec![-1; input.nb_streams() as usize];
    let output: &mut format::context::Output = output_ctx;
    let stream_index = 0;
    for stream in input.streams().into_iter() {
    
        let in_params = stream.parameters();
 
        match in_params.medium() {
            ffmpeg_next::media::Type::Video | ffmpeg_next::media::Type::Audio | ffmpeg_next::media::Type::Subtitle  => {
            stream_list[stream.index()] = stream_index;

            let codec = unsafe {
                Codec::wrap(null_mut())
            };
            // TODO: handle unwrap
            let mut op = output.add_stream(codec).unwrap();
            op.set_parameters(in_params);
            },
            _ => {
            }
        } 
    }
    dump(&output, 0, None);
    stream_list
    
}


fn create_format_out(output: &format::context::Output) -> Format {
    Format::Output(output.format())
}

fn create_new_file<P: AsRef<Path>>(input: &mut format::context::Input,output_ctx: &mut format::context::Output, path: P) -> Vec<i32> {
    if (output_ctx.format().flags() & Flags::NO_FILE).bits() == 0 {
        // let ctx = format::Context::Output(output_ctx);
        open(&path,&create_format_out(output_ctx)).unwrap();
    }
    let stream_index = convert_input_streams(input, output_ctx);

    let mut dict = ffmpeg_next::Dictionary::new();
    dict.set("movflags", "frag_keyframe+empty_moov+default_base_moof");
    let dict_ptr = unsafe { dict.as_mut_ptr() };
    let dict = unsafe {
        ffmpeg_next::Dictionary::own(dict_ptr)
    };

    
    _= output_ctx.write_header_with(dict).unwrap();
    stream_index

}

pub fn new_output_path<P: AsRef<Path>>(path: &P) -> Result<format::Context, ffmpeg_next::Error> {
    let output_ctx = null_mut();
    let format = unsafe {Format::Output(Output::wrap(output_ctx))};
    open(path, &format) 
}

fn main() {
    let mut packet = Packet::empty();
    let mut demuxer = DemuxerCtx::new("./test.mp4").unwrap();
    
    let input_ctx: &mut format::context::Input = &mut demuxer.ctx.input();
    let output = ffmpeg_next::format::Context::Output(unsafe { format::context::Output::wrap(null_mut()) }) ;
    let output_ctx = &mut output.output();
    let stream_index_list = create_new_file(input_ctx, output_ctx,"./its_mpd.mpd");
    let streams_count = input_ctx.nb_streams();
    'stream: loop {
        let opt = packet.read(input_ctx);
        if opt.is_err() {
            opt.unwrap();
            break 'stream;
        }
        let instream = input_ctx.stream(packet.stream()).unwrap();
        if packet.stream() >= streams_count as usize || stream_index_list[packet.stream()] < 0 {
            continue;
        }
        packet.set_stream(stream_index_list[packet.stream()].try_into().unwrap());
        let out_stream = output_ctx.stream(packet.stream()).unwrap();
        let dts =  packet.dts().unwrap();
        let new_dts = dts.rescale_with(instream.time_base(), out_stream.time_base(), Rounding::PassMinMax);
        let pts =packet.pts().unwrap();
        let new_pts = pts.rescale_with(instream.time_base(), out_stream.time_base(),  Rounding::PassMinMax);
        let dur = packet.duration();
        let new_dur = dur.rescale(instream.time_base(), out_stream.time_base());;
        packet.set_dts(Some(new_dts));
        packet.set_pts(Some(new_pts));
        packet.set_duration(new_dur);
        packet.set_position(-1);
        packet.write_interleaved(output_ctx).unwrap();
    };
}
