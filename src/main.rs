use std::{
    ffi::{CString, c_int},
    fs,
    ops::{Deref, DerefMut},
    path::Path,
    ptr::null_mut,
};

use ffmpeg_next::{
    Format, Frame, Packet, Rounding,
    codec::{Context, Parameters},
    decoder::{self, Opened, Video},
    ffi::avcodec_parameters_from_context,
    format::{self, Context as FormatContext, Flags, Input, context::output::dump, open},
    packet,
    software::scaling,
};

use ffmpeg_next::{
    ffi::{AVIO_FLAG_WRITE, avcodec_parameters_copy, avformat_new_stream, avio_open},
    util::mathematics::rescale::Rescale,
};

pub mod encoder;
pub mod demuxer;
pub mod dash;
pub mod muxer;
pub mod transcode;

pub fn rename_mpd_tmp() -> std::io::Result<()> {
    for entry in fs::read_dir(".")? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.ends_with("mpd.tmp") {
            let final_path = Path::new(name.trim_end_matches(".tmp"));
            if final_path.exists() {
                std::fs::remove_file(final_path)?;
            }
            fs::rename(&path, final_path)?;
            println!("Renamed {:?} -> {:?}", path, final_path);
        }
    }
    Ok(())
}

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

impl Deref for DemuxerCtx {
    type Target = FormatContext;

    fn deref(&self) -> &Self::Target {
        &self.ctx
    }
}

impl DerefMut for DemuxerCtx {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.ctx
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
    encoder: &ffmpeg_next::encoder::Encoder,
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

                let ret = match params.medium() {
                    ffmpeg_next::media::Type::Video => {
                        avcodec_parameters_from_context((*out_stream).codecpar, encoder.as_ptr())
                    }

                    ffmpeg_next::media::Type::Audio | ffmpeg_next::media::Type::Subtitle => {
                        avcodec_parameters_copy((*out_stream).codecpar, params.as_ptr())
                    }

                    _ => unreachable!(),
                };

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


fn create_new_file<P: AsRef<Path>>(
    input: &mut format::context::Input,
    output_ctx: &mut format::context::Output,
    path: P,
    encoder: &ffmpeg_next::encoder::Encoder,
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
    let stream_index = convert_input_streams(input, output_ctx, encoder);

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

fn decode_and_encode_frame(
    decoder: &mut Video,
    scaler: &mut scaling::Context,
    encoder: &mut ffmpeg_next::encoder::video::Video,
    stream_index_list: &[i32],
    in_index: usize,
    output_ctx: &mut format::context::Output,
) {
    let mut decoded = ffmpeg_next::util::frame::video::Video::empty();
    let mut encoded_packet = ffmpeg_next::packet::Packet::empty();
    while decoder.receive_frame(&mut decoded).is_ok() {
        let mut new_frame = ffmpeg_next::util::frame::video::Video::empty();
        scaler.run(&decoded, &mut new_frame).unwrap();
        println!(
            "decoded: pts={:?} width={} height={} format={:?}",
            decoded.pts(),
            decoded.width(),
            decoded.height(),
            decoded.format(),
        );

        println!(
            "scaled: pts={:?} width={} height={} format={:?}",
            new_frame.pts(),
            new_frame.width(),
            new_frame.height(),
            new_frame.format(),
        );
        new_frame.set_pts(decoded.pts());
        encoder.send_frame(&new_frame).unwrap();

        while encoder.receive_packet(&mut encoded_packet).is_ok() {
            let out_index = stream_index_list[in_index] as usize;
            encoded_packet.set_stream(out_index);

            let out_stream = output_ctx.stream(encoded_packet.stream()).unwrap();
            if let Some(dts) = encoded_packet.dts() {
                encoded_packet.set_dts(Some(dts.rescale_with(
                    encoder.time_base(),
                    out_stream.time_base(),
                    Rounding::PassMinMax,
                )));
            }
            if let Some(pts) = encoded_packet.pts() {
                encoded_packet.set_pts(Some(pts.rescale_with(
                    encoder.time_base(),
                    out_stream.time_base(),
                    Rounding::PassMinMax,
                )));
            }

            encoded_packet.set_duration(
                encoded_packet
                    .duration()
                    .rescale(encoder.time_base(), out_stream.time_base()),
            );

            encoded_packet.set_position(-1);

            match encoded_packet.write_interleaved(output_ctx) {
                Ok(_) => {}
                Err(e) => {
                    println!("write_interleaved error: {:?}", e);
                    break;
                }
            }
        }
    }
}

fn main() {
    let mut packet = Packet::empty();
    let demuxer = DemuxerCtx::new("./small_bunny_1080p_60fps.mp4").unwrap();
    let input_ctx: &mut format::context::Input = &mut demuxer.ctx.input();
    let codec_ctx = ffmpeg_next::codec::context::Context::from_parameters(
        input_ctx
            .streams()
            .best(ffmpeg_next::media::Type::Video)
            .unwrap()
            .parameters(),
    )
    .unwrap();
    let mut decoder = codec_ctx.decoder().video().unwrap();
    println!("decoder id = {:?}", decoder.id());
    let mut scaler = ffmpeg_next::software::scaling::Context::get(
        decoder.format(),
        decoder.width(),
        decoder.height(),
        decoder.format(),
        1280,
        720,
        ffmpeg_next::software::scaling::Flags::BILINEAR,
    )
    .unwrap();

    let encoder_codec: ffmpeg_next::Codec = ffmpeg_next::codec::encoder::find_by_name("h264_mf").unwrap();
    println!("encoder name = {}", encoder_codec.name());
    println!("encoder id = {:?}", encoder_codec.id());
    let mut encoder: ffmpeg_next::encoder::video::Video =
        ffmpeg_next::codec::context::Context::new_with_codec(encoder_codec)
            .encoder()
            .video()
            .unwrap();

    encoder.set_width(1280);
    encoder.set_height(720);
    encoder.set_format(decoder.format());
    let video_stream = input_ctx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .unwrap();

    encoder.set_time_base(video_stream.time_base());
    encoder.set_frame_rate(Some(video_stream.rate()));
    encoder.set_bit_rate(decoder.bit_rate());
    println!("decoder tb = {:?}", decoder.time_base());
    println!("decoder fr = {:?}", decoder.frame_rate());
    let mut output = ffmpeg_next::format::output("./its_mpd.mpd").unwrap();
    if output
        .format()
        .flags()
        .contains(format::Flags::GLOBAL_HEADER)
    {
        encoder.set_flags(ffmpeg_next::codec::Flags::GLOBAL_HEADER);
    }
    let mut encoder = encoder.open().unwrap();
    let output_ctx: &mut format::context::Output = &mut output;
    let stream_index_list = create_new_file(input_ctx, output_ctx, "./its_mpd.mpd", &encoder);
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
        match instream.parameters().medium() {
            ffmpeg_next::media::Type::Video => {
                decoder.send_packet(&packet).unwrap();
                decode_and_encode_frame(&mut decoder, &mut scaler, &mut encoder, &stream_index_list, in_index, output_ctx)
            }
            ffmpeg_next::media::Type::Audio | ffmpeg_next::media::Type::Subtitle => {
                if packet.stream() >= streams_count as usize
                    || stream_index_list[packet.stream()] < 0
                {
                    continue 'stream;
                }

                let out_index = stream_index_list[in_index] as usize;

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
            _ => {}
        }
    }

    let video_in_index = input_ctx
        .streams()
        .best(ffmpeg_next::media::Type::Video)
        .unwrap()
        .index();

    let video_out_index = stream_index_list[video_in_index] as usize;

    let mut decoded = ffmpeg_next::frame::Video::empty();
    let mut scaled = ffmpeg_next::frame::Video::empty();
    let mut encoded = ffmpeg_next::Packet::empty();

    // Flush decoder
    decoder.send_eof().unwrap();

    while decoder.receive_frame(&mut decoded).is_ok() {
        scaler.run(&decoded, &mut scaled).unwrap();

        println!(
            "decoded: pts={:?} width={} height={} format={:?}",
            decoded.pts(),
            decoded.width(),
            decoded.height(),
            decoded.format(),
        );

        println!(
            "scaled: pts={:?} width={} height={} format={:?}",
            scaled.pts(),
            scaled.width(),
            scaled.height(),
            scaled.format(),
        );
        scaled.set_pts(decoded.pts());

        encoder.send_frame(&scaled).unwrap();

        while encoder.receive_packet(&mut encoded).is_ok() {
            encoded.set_stream(video_out_index);

            let out_stream = output_ctx.stream(video_out_index).unwrap();

            encoded.rescale_ts(encoder.time_base(), out_stream.time_base());

            encoded.set_position(-1);

            encoded.write_interleaved(output_ctx).unwrap();
        }
    }

    // Flush encoder
    encoder.send_eof().unwrap();

    while encoder.receive_packet(&mut encoded).is_ok() {
        encoded.set_stream(video_out_index);

        let out_stream = output_ctx.stream(video_out_index).unwrap();

        encoded.rescale_ts(encoder.time_base(), out_stream.time_base());

        encoded.set_position(-1);

        encoded.write_interleaved(output_ctx).unwrap();
    }
    let err = output_ctx.write_trailer();

    println!("trailer = {:?}", err);

    // std::mem::forget(output);

    println!("finished");

    unsafe {
        println!("pb after trailer = {:?}", (*output_ctx.as_ptr()).pb);
    }
    unsafe {
        println!(
            "libavformat version: {}",
            ffmpeg_next::ffi::avformat_version()
        );
    }

    unsafe {
        let ctx = output_ctx.as_ptr();

        println!("flags = {:x}", (*(*ctx).oformat).flags);
        println!("url = {:?}", std::ffi::CStr::from_ptr((*ctx).url).to_str());
    }
    unsafe {
        let ctx = output_ctx.as_mut_ptr();

        if !ctx.is_null() && ((*(*ctx).oformat).flags & ffmpeg_next::ffi::AVFMT_NOFILE) == 0 {
            if !(*ctx).pb.is_null() {
                let ret = ffmpeg_next::ffi::avio_closep(&mut (*ctx).pb);
                println!("avio_closep -> {}", ret);
            }
        }

        // ffmpeg_next::ffi::avformat_free_context(ctx);
    }
    drop(output);
    #[cfg(windows)]
    rename_mpd_tmp().unwrap()
}
