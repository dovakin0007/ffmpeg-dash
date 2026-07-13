use std::{
    ops::{Deref, DerefMut}, path::Path, ptr::null_mut,
};

use ffmpeg_next::{
    Format, codec, format::{Input, open, context::input::Input as InputContext}, media::Type as MediaType
};

use anyhow::Result;
use thiserror::Error;

use crate::demuxer::DemuxerCtxError::MediaTypeNotFound;

#[derive(Debug, Error)]
pub enum DemuxerCtxError {
    #[error("AV format input failed: {0}")]
    OpenFailed(String),
    #[error("No Codec Parameters found for given Type(MediaType)")]
    MediaTypeNotFound,
}

pub struct DemuxerCtx {
    input: InputContext, 
}

impl DemuxerCtx {
    pub fn new<P: AsRef<Path>>(path: P) -> Result<DemuxerCtx> {
        let av_format_ctx_ptr = null_mut();
        let input = unsafe { Input::wrap(av_format_ctx_ptr) };
        let format = Format::Input(input);
        let ctx = open(&path, &format).map_err(|e| DemuxerCtxError::OpenFailed(e.to_string()))?;
        let input = ctx.input();
        Ok(Self { input })
    }

    pub fn get_best_parameters(&self, r#type: MediaType) -> Result<codec::Parameters> {
        Ok(self.input.streams().best(r#type).ok_or_else(|| {
            MediaTypeNotFound
        })?.parameters())
    }
}

impl Deref for DemuxerCtx {
    type Target = InputContext;
    fn deref(&self) -> &Self::Target {
        &self.input
    }
}

impl DerefMut for DemuxerCtx {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.input
    }
}
