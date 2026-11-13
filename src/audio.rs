//! Playback of cached sounds (OGG, MP3, WAV) through the default output device.

use crate::store::Id;
use anyhow::{anyhow, Result};
use rodio::Source;
use std::io::Cursor;
use std::time::Duration;

pub struct Audio {
    // Dropping the sink closes the device, so it lives as long as the player.
    _sink: rodio::MixerDeviceSink,
    player: rodio::Player,
    pub current: Option<Id>,
    pub duration: Option<Duration>,
    volume: f32,
}

impl Audio {
    pub fn open() -> Result<Self> {
        let mut sink = rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| anyhow!("no audio output: {e}"))?;
        sink.log_on_drop(false);
        let player = rodio::Player::connect_new(sink.mixer());
        Ok(Self { _sink: sink, player, current: None, duration: None, volume: 0.8 })
    }

    /// Starts playing `data` from the beginning, replacing whatever was loaded.
    pub fn play(&mut self, id: Id, data: Vec<u8>) -> Result<()> {
        let source = rodio::Decoder::try_from(Cursor::new(data)).map_err(|e| anyhow!("cannot decode this sound: {e}"))?;
        self.duration = source.total_duration();
        self.player.stop();
        self.player.set_volume(self.volume);
        self.player.append(source);
        self.player.play();
        self.current = Some(id);
        Ok(())
    }

    pub fn stop(&mut self) {
        self.player.stop();
        self.current = None;
    }

    pub fn toggle_pause(&self) {
        if self.player.is_paused() {
            self.player.play();
        } else {
            self.player.pause();
        }
    }

    /// True while a sound is loaded and has not finished.
    pub fn active(&self) -> bool {
        self.current.is_some() && !self.player.empty()
    }

    pub fn paused(&self) -> bool {
        self.player.is_paused()
    }

    pub fn position(&self) -> Duration {
        self.player.get_pos()
    }

    pub fn seek(&self, to: Duration) {
        // Not every stream can seek; a refusal just leaves playback where it was.
        let _ = self.player.try_seek(to);
    }

    pub fn volume(&self) -> f32 {
        self.volume
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        self.player.set_volume(volume);
    }
}

pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}
