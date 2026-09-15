//! Symphonia integration for reading audio files.

use super::wave::*;
use std::fs::File;
use std::io::Cursor;
use std::path::Path;
extern crate alloc;
use alloc::boxed::Box;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::registry::CodecRegistry;
use symphonia::core::errors::{Error, Result};
use symphonia::core::formats::FormatOptions;
use symphonia::core::formats::probe::{Hint, Probe};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;

pub type WaveResult<T> = Result<T>;
pub type WaveError = Error;

impl Wave {
    /// Load first track of audio file from the given path.
    /// Supported formats are anything that Symphonia can read.
    pub fn load<P: AsRef<Path>>(path: P) -> WaveResult<Wave> {
        Wave::load_track(path, None)
    }

    /// Load first track of audio from the given slice.
    /// Supported formats are anything that Symphonia can read.
    pub fn load_slice<S: AsRef<[u8]> + Send + Sync + 'static>(slice: S) -> WaveResult<Wave> {
        Wave::load_slice_track(slice, None)
    }

    /// Load audio from the given slice. Track can be optionally selected.
    /// If not selected, the first track with a known codec will be loaded.
    /// Supported formats are anything that Symphonia can read.
    pub fn load_slice_track<S: AsRef<[u8]> + Send + Sync + 'static>(
        slice: S,
        track: Option<usize>,
    ) -> WaveResult<Wave> {
        let hint = Hint::new();
        let source: Box<dyn MediaSource> = Box::new(Cursor::new(slice));
        Wave::decode(source, track, hint)
    }

    /// Load audio file from the given path. Track can be optionally selected.
    /// If not selected, the first track with a known codec will be loaded.
    /// Supported formats are anything that Symphonia can read.
    pub fn load_track<P: AsRef<Path>>(path: P, track: Option<usize>) -> WaveResult<Wave> {
        let path = path.as_ref();
        let mut hint = Hint::new();

        if let Some(extension) = path.extension() {
            if let Some(extension_str) = extension.to_str() {
                hint.with_extension(extension_str);
            }
        }

        let source: Box<dyn MediaSource> = match File::open(path) {
            Ok(file) => Box::new(file),
            Err(error) => return Err(Error::IoError(error)),
        };

        Wave::decode(source, track, hint)
    }

    /// Decode track from the given source, with the given registries for codecs and formats
    pub fn decode_with(source: Box<dyn MediaSource>, track: Option<usize>, hint: Hint, codec_registry: &CodecRegistry, probe: &Probe) -> WaveResult<Wave> {
        let stream = MediaSourceStream::new(source, Default::default());

        let mut format_opts = FormatOptions::default();

        let metadata_opts: MetadataOptions = Default::default();

        let mut wave: Option<Wave> = None;

        let mut probed = symphonia::default::get_probe().probe(&hint, stream, format_opts, metadata_opts)?;
        // Select track if specified, otherwise select the first track with a known codec.
        let Some((track, track_codec)) =
            // Get a slice of tracks
            track.map(|idx| probed.tracks().get(idx).map_or([].as_ref(), std::slice::from_ref)).unwrap_or_else(|| {
                probed
                .tracks()
            })
            .iter()
            .filter_map(|t| t.codec_params.as_ref().and_then(|i| i.audio()).map(|codec| (t, codec)))
            .next()
        else { return Err(Error::DecodeError("Could not find track.")); };

        let track_id = track.id;

        let decode_opts = AudioDecoderOptions::default();

        let mut decoder = codec_registry.make_audio_decoder(track_codec, &decode_opts)?;

        loop {
            let packet = match probed.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => {
                    if let Some(wave_output) = wave {
                        return Ok(wave_output);
                    } else {
                        // This is the closest error I can think of
                        return Err(Error::SeekError(symphonia::core::errors::SeekErrorKind::OutOfRange));
                    }
                },
                Err(err) => {
                    if let Some(wave_output) = wave {
                        return Ok(wave_output);
                    } else {
                        return Err(err);
                    }
                }
            };

            // If the packet does not belong to the selected track, skip it.
            if packet.track_id != track_id {
                continue;
            }

            let decoded = decoder.decode(&packet)?;
            if wave.is_none() {
                let spec = decoded.spec();
                wave = Some(Wave::new(spec.channels().count(), spec.rate() as f64));
            } else {
                // TODO: Check that audio spec hasn't changed.
            }

            if let Some(ref mut wave_output) = wave {
                let buffer_len = decoded.frames();
                let old_len = wave_output.len();
                wave_output.resize(old_len + buffer_len);
                // We can't reuse this buffer because the lifetimes are leaked, and `Vec::recycle` isn't stable yet
                let mut channels: Vec<_> =
                    wave_output.channels_slice_mut()
                    .map(|channel| &mut channel[old_len..])
                    .collect();
                decoded.copy_to_slice_planar(channels.as_mut_slice());
            }
        }
    }

    /// Decode track from the given source.
    fn decode(source: Box<dyn MediaSource>, track: Option<usize>, hint: Hint) -> WaveResult<Wave> {
        Self::decode_with(source, track, hint, symphonia::default::get_codecs(), symphonia::default::get_probe())
    }
}
