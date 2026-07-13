use std::ptr::null_mut;

use ffmpeg_next::{Codec, ffi::av_codec_iterate};

pub struct Encoder {
    codec: Codec,
}

impl Encoder {
    pub fn new(codec_name: Option<&str>) -> Encoder {
        match codec_name {
            _ => {

            }
            None => {
                unsafe {
                    let mut _av_codec_opaque = null_mut();
                    let codec = av_codec_iterate(_av_codec_opaque as *mut *mut libc::c_void);
                   *&(*codec).type_ == 
                    
                }
            }
        }
        todo!()
    }
}
