use std::{
    error::Error,
    ffi::{CString, c_int},
    ops::{Deref, DerefMut},
    path::Path,
    ptr::null_mut,
};

use ffmpeg_next::{
    Codec, Format, Frame, Packet, Rounding, Stream,
    codec::{Context, Parameters},
    decoder::{self, Opened},
    device::input,
    ffi::{avformat_close_input, avformat_free_context, avio_close, avio_closep},
    format::{
        self, Context as FormatContext, Flags, Input, Output,
        context::{Destructor, output::dump},
        open,
    },
    packet,
};
use ffmpeg_next::{
    ffi::{AVIO_FLAG_WRITE, avcodec_parameters_copy, avformat_new_stream, avio_open},
    util::mathematics::rescale::Rescale,
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
    pub frame: Frame,
}

impl WrappedFrame {
    pub fn new() -> WrappedFrame {
        let frame = unsafe { Frame::empty() };
        Self { frame }
    }
    pub fn is_video(&self) -> bool {
        unsafe { self.as_ptr().read().width > 0 }
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
fn convert_input_streams(
    input: &mut format::context::Input,
    output: &mut format::context::Output,
) -> Vec<i32> {
    let mut stream_list = vec![-1; input.nb_streams() as usize];
    let mut stream_index = 0;

    let out_ctx = unsafe { output.as_mut_ptr() };

    for stream in input.streams() {
        let params = stream.parameters();

        match params.medium() {
            ffmpeg_next::media::Type::Video
            | ffmpeg_next::media::Type::Audio
            | ffmpeg_next::media::Type::Subtitle => unsafe {
                let out_stream = avformat_new_stream(out_ctx, std::ptr::null());

                if out_stream.is_null() {
                    panic!("avformat_new_stream failed");
                }

                let ret = avcodec_parameters_copy((*out_stream).codecpar, params.as_ptr());

                if ret < 0 {
                    panic!("avcodec_parameters_copy failed: {}", ret);
                }

                (*out_stream).codecpar.as_mut().unwrap().codec_tag = 0;

                (*out_stream).time_base.num = stream.time_base().numerator();
                (*out_stream).time_base.den = stream.time_base().denominator();

                (*out_stream).avg_frame_rate.num = stream.avg_frame_rate().numerator();
                (*out_stream).avg_frame_rate.den = stream.avg_frame_rate().denominator();

                (*out_stream).r_frame_rate.num = stream.rate().numerator();
                (*out_stream).r_frame_rate.den = stream.rate().denominator();

                println!(
                    "codec={:?} extradata={} codec_tag={}",
                    params.id(),
                    (*(*out_stream).codecpar).extradata_size,
                    (*(*out_stream).codecpar).codec_tag,
                );

                stream_list[stream.index()] = stream_index;
                stream_index += 1;
            },
            _ => {}
        }
    }

    dump(output, 0, None);

    stream_list
}

fn create_format_out(output: &format::context::Output) -> Format {
    Format::Output(output.format())
}

fn create_new_file<P: AsRef<Path>>(
    input: &mut format::context::Input,
    output_ctx: &mut format::context::Output,
    path: P,
) -> Vec<i32> {
    if (output_ctx.format().flags() & Flags::NO_FILE).bits() == 0 {
        let filename = CString::new(path.as_ref().to_str().unwrap()).unwrap();

        unsafe {
            let ctx = output_ctx.as_mut_ptr();

            let ret = avio_open(&mut (*ctx).pb, filename.as_ptr(), AVIO_FLAG_WRITE);

            if ret < 0 {
                panic!("avio_open failed: {}", ret);
            }
        }
    }
    let stream_index = convert_input_streams(input, output_ctx);

    for (i, stream) in input.streams().enumerate() {
        println!("Input stream {} TB = {:?}", i, stream.time_base());
    }

    let dict = ffmpeg_next::Dictionary::new();
    // dict.set("movflags", "frag_keyframe+empty_moov+default_base_moof");
    unsafe {
        println!("Before write_header:");
        println!("pb = {:?}", (*output_ctx.as_ptr()).pb);

        for (i, stream) in output_ctx.streams().enumerate() {
            println!("Output stream {} TB: {:?}", i, stream.time_base());

            println!("Codec TB: {:?}", stream.parameters().medium());
        }
    }
    unsafe {
        let mut opts = dict.disown();

        let ret = ffmpeg_next::ffi::avformat_write_header(output_ctx.as_mut_ptr(), &mut opts);

        if ret < 0 {
            panic!("avformat_write_header failed: {}", ret);
        }
    }
    println!("After write_header");

    for (i, stream) in output_ctx.streams().enumerate() {
        println!("Output stream {} TB = {:?}", i, stream.time_base());
    }
    println!("Muxer: {}", output_ctx.format().name());

    stream_index
}

fn main() {
    let mut packet = Packet::empty();
    let demuxer = DemuxerCtx::new("./small_bunny_1080p_60fps.mp4").unwrap();

    let input_ctx: &mut format::context::Input = &mut demuxer.ctx.input();
    let mut output = ffmpeg_next::format::output("./its_mpd.mpd").unwrap();
    let output_ctx = &mut output;
    let stream_index_list = create_new_file(input_ctx, output_ctx, "./its_mpd.mpd");
    let streams_count = input_ctx.nb_streams();
    'stream: loop {
        match packet.read(input_ctx) {
            Ok(_) => {}
            Err(ffmpeg_next::Error::Eof) => {
                println!("EOF");
                break;
            }
            Err(e) => {
                println!("Read error: {:?}", e);
                break;
            }
        }
        let in_index = packet.stream();

        let instream = input_ctx.stream(in_index).unwrap();
        if packet.stream() >= streams_count as usize || stream_index_list[packet.stream()] < 0 {
            continue 'stream;
        }

        let out_index = stream_index_list[in_index] as usize;

        let out_stream = output_ctx.stream(out_index).unwrap();

        packet.set_stream(out_index);
        let out_stream = output_ctx.stream(packet.stream()).unwrap();
        if let Some(dts) = packet.dts() {
            packet.set_dts(Some(dts.rescale_with(
                instream.time_base(),
                out_stream.time_base(),
                Rounding::PassMinMax,
            )));
        }
        if let Some(pts) = packet.pts() {
            packet.set_pts(Some(pts.rescale_with(
                instream.time_base(),
                out_stream.time_base(),
                Rounding::PassMinMax,
            )));
        }

        packet.set_duration(
            packet
                .duration()
                .rescale(instream.time_base(), out_stream.time_base()),
        );

        packet.set_position(-1);

        match packet.write_interleaved(output_ctx) {
            Ok(_) => {}
            Err(e) => {
                println!("write_interleaved error: {:?}", e);
                break;
            }
        }
    }
    let err = output_ctx.write_trailer();

    println!("trailer = {:?}", err);

    // std::mem::forget(output);

    println!("finished");

    // unsafe {
    //     println!("pb after trailer = {:?}", (*output_ctx.as_ptr()).pb);
    // }
    // unsafe {
    //     println!(
    //         "libavformat version: {}",
    //         ffmpeg_next::ffi::avformat_version()
    //     );
    // }

    // unsafe {
    //     let ctx = output_ctx.as_ptr();

    //     println!("flags = {:x}", (*(*ctx).oformat).flags);
    //     println!("url = {:?}", std::ffi::CStr::from_ptr((*ctx).url).to_str());
    // }
    // unsafe {
    //     let ctx = output_ctx.as_mut_ptr();

    //     if !ctx.is_null() && ((*(*ctx).oformat).flags & ffmpeg_next::ffi::AVFMT_NOFILE) == 0 {
    //         if !(*ctx).pb.is_null() {
    //             let ret = ffmpeg_next::ffi::avio_closep(&mut (*ctx).pb);
    //             println!("avio_closep -> {}", ret);
    //         }
    //     }

    //     ffmpeg_next::ffi::avformat_free_context(ctx);
    // }
}
