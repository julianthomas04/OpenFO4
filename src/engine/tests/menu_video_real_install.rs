//! The converted main menu video plays through Windows Media Foundation: it decodes at the right
//! size and colour, paces itself in real time, loops without a gap and stops on request. Ignored by
//! default (needs an installation). Run with `OPENFO4_INSTALL_DIR` set:
//! `cargo test --test menu_video_real_install -- --ignored --nocapture --test-threads=1`

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use openfo4::assets::{AssetRootEnv, layout, resolve_asset_root};
use openfo4::video::{
    ColourMatrix, ColourRange, DecodedFrame, MAIN_MENU_VIDEO_SOURCE, MenuVideo, MenuVideoSettings,
    Pacing, Player, YuvToRgb,
};

const SIZE: (u32, u32) = (1920, 1080);
const FRAME_RATE: f64 = 30.0;
const FRAME_PERIOD: Duration = Duration::from_nanos(33_333_333);
const TIMESTAMP_TOLERANCE: Duration = Duration::from_millis(2);
/// The file's first frame may not be at 0: the encoder can delay the presentation by its frame
/// reordering.
const MAX_START_OFFSET: Duration = Duration::from_millis(100);
const WAIT: Duration = Duration::from_secs(30);

fn menu_video() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let assets = resolve_asset_root(&AssetRootEnv::from_process(), exe.parent().unwrap()).unwrap();
    PathBuf::from(layout::video_path(
        &assets.to_string_lossy(),
        MAIN_MENU_VIDEO_SOURCE,
    ))
}

fn play(video: &std::path::Path, pacing: Pacing) -> (Player, Receiver<DecodedFrame>) {
    let (sender, receiver) = mpsc::sync_channel(4096);
    let player = Player::start_paced(video, pacing, move |frame| {
        let _ = sender.try_send(frame);
    })
    .expect("the menu video starts");
    (player, receiver)
}

fn next(receiver: &Receiver<DecodedFrame>, player: &Player) -> DecodedFrame {
    let frame = receiver.recv_timeout(WAIT);
    if let Some(error) = player.take_failure() {
        panic!("{error}");
    }
    frame.expect("a frame arrives")
}

#[test]
#[ignore = "needs the installed menu video"]
fn the_first_frames_have_the_converted_size_colour_and_picture() {
    let video = menu_video();
    let (player, receiver) = play(&video, Pacing::Unthrottled);

    let mut brightest = 0_u8;
    for index in 0..30 {
        let frame = next(&receiver, &player);
        assert_eq!(frame.size(), SIZE, "frame {index}");
        assert_eq!(
            frame.conversion(),
            YuvToRgb::new(ColourMatrix::Bt601, ColourRange::Limited),
            "the converter tags the video BT.601 limited range"
        );
        let planes = frame.planes();
        let width = planes.width as usize;
        let luma_max = (0..planes.height as usize)
            .flat_map(|row| {
                let start = row * planes.planes[0].stride;
                planes.planes[0].data[start..start + width].iter().copied()
            })
            .max()
            .unwrap_or(0);
        brightest = brightest.max(luma_max);
        if index == 0 {
            assert!(frame.timing().stream_time <= MAX_START_OFFSET);
        }
    }
    assert!(
        brightest > 64,
        "the picture is not black: brightest luma {brightest}"
    );
    player.shutdown();
}

#[test]
#[ignore = "needs the installed menu video"]
fn playback_is_paced_in_real_time() {
    let video = menu_video();
    let (player, receiver) = play(&video, Pacing::Clock);
    let first = next(&receiver, &player);
    assert!(first.timing().stream_time <= MAX_START_OFFSET);
    let started = Instant::now();

    let mut count = 0;
    while started.elapsed() < Duration::from_secs(3) {
        if receiver.recv_timeout(Duration::from_millis(200)).is_ok() {
            count += 1;
        }
    }
    player.shutdown();

    let expected = 3.0 * FRAME_RATE;
    assert!(
        (f64::from(count) - expected).abs() <= 3.0,
        "{count} frames in 3 s of real time, expected about {expected}"
    );
}

#[test]
#[ignore = "needs the installed menu video"]
fn the_loop_wraps_without_a_gap_on_the_clock_timeline() {
    let video = menu_video();
    let (player, receiver) = play(&video, Pacing::Unthrottled);

    let mut previous = next(&receiver, &player).timing();
    let round_start = previous.stream_time;
    let mut wraps = 0;
    let mut frames_in_round = 1_u32;
    let mut round_lengths = Vec::new();
    while wraps < 3 {
        let timing = next(&receiver, &player).timing();
        let (stream, running) = (timing.stream_time, timing.running_time);
        let (last_stream, last_running) = (previous.stream_time, previous.running_time);
        assert!(
            running.abs_diff(last_running + FRAME_PERIOD) <= TIMESTAMP_TOLERANCE,
            "clock timeline stepped {last_running:?} -> {running:?} (a gap or a jump)"
        );
        if stream < last_stream {
            assert!(
                stream.abs_diff(round_start) <= TIMESTAMP_TOLERANCE,
                "the loop restarts at the first frame: {stream:?} vs {round_start:?}"
            );
            wraps += 1;
            round_lengths.push(frames_in_round);
            frames_in_round = 0;
        } else {
            assert!(
                stream.abs_diff(last_stream + FRAME_PERIOD) <= TIMESTAMP_TOLERANCE,
                "file timeline stepped {last_stream:?} -> {stream:?}"
            );
        }
        frames_in_round += 1;
        previous = timing;
    }
    player.shutdown();

    assert!(
        round_lengths.windows(2).all(|pair| pair[0] == pair[1]),
        "every round has the same frame count: {round_lengths:?}"
    );
    println!("frames per loop: {}", round_lengths[0]);
}

#[test]
#[ignore = "needs the installed menu video"]
fn shutting_down_stops_the_frames_and_returns_promptly() {
    let video = menu_video();
    let (player, receiver) = play(&video, Pacing::Clock);
    next(&receiver, &player);

    let started = Instant::now();
    player.shutdown();
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "teardown took {:?}",
        started.elapsed()
    );

    while receiver.try_recv().is_ok() {}
    std::thread::sleep(Duration::from_millis(300));
    assert!(receiver.try_recv().is_err(), "no frame after the shutdown");
}

/// A start this far into the file reaches the loop point within a few seconds; the converted
/// video is longer than this.
const NEAR_THE_END: Duration = Duration::from_secs(130);
/// A frame that arrives later than two frame periods after its predecessor is a visible hitch.
const MAX_DELIVERY_GAP: Duration = Duration::from_nanos(66_666_666);

#[test]
#[ignore = "needs the installed menu video"]
fn real_time_playback_does_not_hitch_at_the_loop_point() {
    let video = menu_video();
    let (sender, receiver) = mpsc::sync_channel(4096);
    let player = Player::start_from(&video, Pacing::Clock, NEAR_THE_END, move |frame| {
        let _ = sender.try_send((Instant::now(), frame.timing().stream_time));
    })
    .expect("the menu video starts");

    let mut previous: Option<(Instant, Duration)> = None;
    let mut worst = Duration::ZERO;
    let mut wrapped = false;
    let deadline = Instant::now() + Duration::from_secs(40);
    while Instant::now() < deadline {
        if let Some(error) = player.take_failure() {
            panic!("{error}");
        }
        let Ok((at, stream)) = receiver.recv_timeout(Duration::from_secs(2)) else {
            continue;
        };
        if let Some((last_at, last_stream)) = previous {
            if stream < last_stream {
                wrapped = true;
            }
            worst = worst.max(at - last_at);
        }
        previous = Some((at, stream));
        if wrapped && stream > Duration::from_secs(2) {
            break;
        }
    }
    player.shutdown();

    assert!(wrapped, "the video never reached its loop point");
    println!("worst delivery gap: {worst:?}");
    assert!(
        worst < MAX_DELIVERY_GAP,
        "frames were delivered {worst:?} apart"
    );
}

fn assets_with_video(content: &[u8]) -> tempfile::TempDir {
    let directory = tempfile::tempdir().unwrap();
    let video = directory.path().join("video");
    std::fs::create_dir_all(&video).unwrap();
    std::fs::write(video.join("mainmenuloop.mp4"), content).unwrap();
    directory
}

fn enabled() -> MenuVideoSettings {
    MenuVideoSettings { enabled: true }
}

#[test]
#[ignore = "needs Media Foundation"]
fn a_damaged_file_is_reported_once_and_leaves_the_menu_without_video() {
    let assets = assets_with_video(&[0x42; 4096]);
    let mut menu_video = MenuVideo::new(enabled(), assets.path());

    let deadline = Instant::now() + WAIT;
    let error = loop {
        match menu_video.update(true) {
            Err(error) => break error,
            Ok(_) => assert!(
                Instant::now() < deadline,
                "no error for a file that is not a video"
            ),
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    menu_video.abandon(&error);

    assert_eq!(menu_video.update(true).unwrap(), None, "no retry");
    assert!(menu_video.take_frame().is_none());
}

#[test]
#[ignore = "needs the installed menu video"]
fn no_frame_of_a_stopped_run_reaches_the_next_one() {
    let exe = std::env::current_exe().unwrap();
    let assets = resolve_asset_root(&AssetRootEnv::from_process(), exe.parent().unwrap()).unwrap();
    let mut menu_video = MenuVideo::new(enabled(), &assets);

    menu_video.update(true).unwrap();
    let deadline = Instant::now() + WAIT;
    while menu_video.take_frame().is_none() {
        assert!(Instant::now() < deadline, "no first frame");
        std::thread::sleep(Duration::from_millis(10));
    }
    menu_video.update(false).unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        menu_video.take_frame().is_none(),
        "a frame arrived after the stop"
    );

    menu_video.update(true).unwrap();
    let deadline = Instant::now() + WAIT;
    let first = loop {
        if let Some(frame) = menu_video.take_frame() {
            break frame;
        }
        assert!(Instant::now() < deadline, "no frame after the restart");
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(
        first.timing().stream_time <= MAX_START_OFFSET,
        "the first frame after a restart is not from the start: {:?}",
        first.timing()
    );
}
