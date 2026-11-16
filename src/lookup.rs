//! Looking a single asset ID up: what Roblox says it is, the file itself when
//! Roblox hands it out, and which cached entry it is.
//!
//! Sounds are the hard case. Roblox gives most of them only to players who are
//! logged in and in the game, so the file cannot be fetched here and nothing
//! ties the ID to an entry. What is public is the sound's name and its length
//! in whole seconds. That narrows the cached sounds down to a handful, and the
//! user picks the right one by ear; from then on the ID is known like any other.

use crate::config::{fingerprint, IdMap};
use crate::format;
use crate::resolve::{self, Outcome};
use crate::store::{Entry, Id, Store};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

/// The asset type number of a sound.
pub const AUDIO: u32 = 3;

/// What Roblox tells anyone about an asset.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    pub name: String,
    pub creator: String,
    pub type_id: Option<u32>,
    /// For a sound: its length, in whole seconds.
    pub seconds: Option<u32>,
}

fn get_json(url: &str) -> Option<serde_json::Value> {
    let response = ureq::get(url).set("User-Agent", "rbxcache").timeout(Duration::from_secs(15)).call().ok()?;
    serde_json::from_reader(response.into_reader()).ok()
}

/// Asks Roblox's public catalogue about an asset. `None` when it has nothing to say.
pub fn details(id: u64) -> Option<Details> {
    let found = get_json(&format!("https://economy.roblox.com/v2/assets/{id}/details"))?;
    let mut details = Details {
        name: found["Name"].as_str()?.to_string(),
        creator: found["Creator"]["Name"].as_str().unwrap_or_default().to_string(),
        type_id: found["AssetTypeId"].as_u64().map(|n| n as u32),
        seconds: None,
    };
    if details.type_id == Some(AUDIO) {
        let more = get_json(&format!("https://apis.roblox.com/toolbox-service/v1/items/details?assetIds={id}"));
        details.seconds = more.and_then(|more| more["data"][0]["asset"]["duration"].as_u64()).map(|n| n as u32);
    }
    Some(details)
}

/// The asset IDs in text someone typed or pasted: bare numbers, `rbxassetid://`
/// addresses and links all count. Each ID is given once, in the order met.
pub fn parse_ids(text: &str) -> Vec<u64> {
    let mut ids = Vec::new();
    for run in text.split(|c: char| !c.is_ascii_digit()) {
        // Shorter runs are the digits of something else: a version number, a port.
        if let Some(id) = run.parse::<u64>().ok().filter(|_| run.len() >= 5) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

/// One asset ID, looked up.
pub struct Found {
    pub id: u64,
    pub details: Option<Details>,
    /// How matching it to the cache went.
    pub outcome: Outcome,
    /// The file and the kind of content it is, when Roblox handed it out.
    pub file: Option<(&'static str, Arc<Vec<u8>>)>,
}

impl Found {
    pub fn is_sound(&self) -> bool {
        self.details.as_ref().is_some_and(|d| d.type_id == Some(AUDIO))
    }
}

/// Looks `ids` up afresh. What is learned about their content goes into `idmap`.
pub fn find(store: &Store, entries: &[Entry], idmap: &mut IdMap, ids: &[u64], progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<Found> {
    let outcomes = resolve::identify(store, entries, idmap, ids, true, progress);
    outcomes
        .into_iter()
        .map(|(id, outcome)| {
            // An ID tagged by hand stays tagged, whatever Roblox says about it now.
            let tagged = idmap.assets.get(&id.to_string()).is_some_and(|prints| !prints.is_empty());
            let outcome = match outcome {
                Outcome::Denied(_) if tagged => Outcome::NotCached,
                other => other,
            };
            let file = match (&outcome, resolve::fetch(id, false)) {
                (Outcome::Denied(_) | Outcome::Gone, _) => None,
                (_, Ok(resolve::Fetched::Body(body))) => Some((format::sniff(&body), Arc::new(body))),
                _ => None,
            };
            Found { id, details: details(id), outcome, file }
        })
        .collect()
}

/// A cached sound, as a candidate for an ID that could not be fetched.
#[derive(Clone, Debug, PartialEq)]
pub struct Sound {
    pub id: Id,
    pub size: u64,
    /// How long it plays, when that could be read from the file.
    pub seconds: Option<f32>,
    /// Last use, milliseconds since the Unix epoch.
    pub atime: i64,
    /// The asset ID it is already tied to.
    pub taken: Option<String>,
    /// The MD5 of its content: the cache often holds the same sound under several entries.
    pub md5: String,
}

/// Every sound in the cache, with its length and whether an ID is tied to it.
pub fn sounds(store: &Store, entries: &[Entry], idmap: &IdMap) -> Vec<Sound> {
    entries
        .iter()
        .filter(|e| e.wrapped && format::is_audio(e.kind))
        .filter_map(|e| {
            let body = store.read_body(&e.id).ok()?;
            let print = fingerprint(&body);
            let taken = idmap.assets_for(&print).first().map(|id| id.to_string());
            Some(Sound { id: e.id, size: e.body_len, seconds: seconds(&body), atime: e.atime, taken, md5: print.md5 })
        })
        .collect()
}

/// How long a sound plays. Ogg files are read directly: the last page says
/// how many samples there are. Anything else is asked of the decoder.
pub fn seconds(body: &[u8]) -> Option<f32> {
    if body.starts_with(b"OggS") {
        let find = |needle: &[u8]| body.windows(needle.len()).position(|w| w == needle);
        let rate = match (find(b"\x01vorbis"), find(b"OpusHead")) {
            (Some(at), _) => u32::from_le_bytes(body.get(at + 12..at + 16)?.try_into().ok()?),
            // Opus counts in 48 kHz samples whatever the file was recorded at.
            (None, Some(_)) => 48_000,
            _ => return None,
        };
        let last = body.windows(4).rposition(|w| w == b"OggS")?;
        let samples = i64::from_le_bytes(body.get(last + 6..last + 14)?.try_into().ok()?);
        return (rate > 0 && samples > 0).then(|| samples as f32 / rate as f32);
    }
    use rodio::Source;
    let decoder = rodio::Decoder::try_from(Cursor::new(body.to_vec())).ok()?;
    decoder.total_duration().map(|d| d.as_secs_f32())
}

/// The cached sounds that could be the one Roblox describes as `about`
/// seconds long, likeliest first. Roblox gives the length in whole seconds, so
/// anything within a second counts. Sounds already tied to an ID are left out.
/// With no length to go by, every free sound is a candidate, newest first.
/// Copies of the same sound are listed once: tying an ID to one ties it to all.
pub fn candidates(sounds: &[Sound], about: Option<u32>) -> Vec<&Sound> {
    let mut seen = std::collections::HashSet::new();
    let mut free: Vec<&Sound> = sounds.iter().filter(|s| s.taken.is_none() && seen.insert((s.md5.as_str(), s.size))).collect();
    match about {
        Some(about) => {
            let off = |s: &Sound| s.seconds.map(|seconds| (seconds - about as f32).abs());
            free.retain(|s| off(s).is_none_or(|off| off <= 1.0));
            free.sort_by(|a, b| off(a).unwrap_or(1.0).total_cmp(&off(b).unwrap_or(1.0)).then(b.atime.cmp(&a.atime)));
        }
        None => free.sort_by_key(|s| std::cmp::Reverse(s.atime)),
    }
    free
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_picked_out_of_whatever_was_pasted() {
        let text = "16530229616, rbxassetid://16530229541\nhttps://www.roblox.com/library/16737738420/Slide v1.2 16530229616 port 8080";
        assert_eq!(parse_ids(text), [16530229616, 16530229541, 16737738420]);
        assert!(parse_ids("nothing here, 1234").is_empty());
    }

    /// The pages of a Vorbis file with this many samples at this rate: only what `seconds` reads.
    fn ogg(rate: u32, samples: i64) -> Vec<u8> {
        let mut body = b"OggS\0\x02".to_vec();
        body.extend_from_slice(&[0; 22]);
        body.extend_from_slice(b"\x01vorbis\0\0\0\0\x02");
        body.extend_from_slice(&rate.to_le_bytes());
        body.extend_from_slice(&[0; 40]);
        body.extend_from_slice(b"OggS\0\x04");
        body.extend_from_slice(&samples.to_le_bytes());
        body.extend_from_slice(&[0; 12]);
        body
    }

    #[test]
    fn the_last_page_of_an_ogg_file_gives_its_length() {
        assert_eq!(seconds(&ogg(44_100, 66_150)), Some(1.5));
        assert_eq!(seconds(&ogg(48_000, 0)), None);
        assert_eq!(seconds(b"OggS but nothing else"), None);
        assert_eq!(seconds(b"not a sound"), None);
    }

    #[test]
    fn candidates_are_the_free_sounds_of_about_that_length() {
        let sound = |n: u8, seconds: Option<f32>, atime: i64, taken: bool| Sound {
            id: [n; 16],
            size: 1,
            seconds,
            atime,
            taken: taken.then(|| "9".to_string()),
            md5: n.to_string(),
        };
        let sounds = [
            sound(1, Some(2.4), 10, false),
            sound(2, Some(1.9), 5, false),
            sound(3, Some(2.0), 7, true),
            sound(4, Some(7.0), 9, false),
            sound(5, None, 8, false),
        ];
        let ids = |found: Vec<&Sound>| found.iter().map(|s| s.id[0]).collect::<Vec<_>>();
        // Closest first; one of unknown length might be it, so it stays, last among the plausible.
        assert_eq!(ids(candidates(&sounds, Some(2))), [2, 1, 5]);
        assert_eq!(ids(candidates(&sounds, None)), [1, 4, 5, 2]);
        // A second copy of a sound adds nothing to choose from.
        let twice = [sounds[1].clone(), Sound { id: [9; 16], ..sounds[1].clone() }];
        assert_eq!(ids(candidates(&twice, Some(2))), [2]);
    }
}
