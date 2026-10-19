//! Command-line front end: `mp3enc input.wav output.mp3 [--bitrate KBPS] [--mono|--stereo] [--shaping off|ath|psy]`

use std::process::ExitCode;

use mp3enc::{Encoder, EncoderConfig, NoiseShaping, StereoMode, wav};

const USAGE: &str = "usage: mp3enc <input.wav> <output.mp3> [--bitrate KBPS] [--mono | --stereo] [--shaping off|ath|psy]";

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut positional = Vec::new();
    let mut bitrate = 192;
    let mut stereo = StereoMode::Joint;
    let mut shaping = None;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-b" | "--bitrate" => {
                bitrate = it.next().ok_or("--bitrate needs a value")?.parse()?;
            }
            "--mono" => stereo = StereoMode::Mono,
            "--stereo" => stereo = StereoMode::Stereo,
            "--shaping" => {
                shaping = Some(match it.next().map(String::as_str) {
                    Some("off") => NoiseShaping::Off,
                    Some("ath") => NoiseShaping::AbsoluteThreshold,
                    Some("psy") => NoiseShaping::Psychoacoustic,
                    _ => return Err(format!("--shaping needs off, ath or psy\n{USAGE}").into()),
                });
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            s if s.starts_with('-') => return Err(format!("unknown option {s}\n{USAGE}").into()),
            _ => positional.push(arg.clone()),
        }
    }
    let [input, output] = positional.as_slice() else {
        return Err(USAGE.into());
    };

    let audio = wav::read(input)?;
    let mut config = EncoderConfig::cbr(audio.sample_rate, audio.channels.len(), bitrate);
    config.stereo = stereo;
    if let Some(shaping) = shaping {
        config.noise_shaping = shaping;
    }
    let mut encoder = Encoder::new(config)?;

    let planes: Vec<&[f32]> = audio.channels.iter().map(Vec::as_slice).collect();
    let mut mp3 = encoder.encode(&planes)?;
    mp3.extend(encoder.finish()?);
    std::fs::write(output, mp3)?;
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mp3enc: {e}");
            ExitCode::FAILURE
        }
    }
}
