//! The fake platform must honour the same contracts the real adapters will:
//! fixture-driven topologies, one frame per display in ascending id order,
//! `NoDisplays` for an empty topology, `DisplayFailed` without partial output,
//! and an overlay host that records its calls.

use snapshade_core::capture::{CaptureError, CaptureRequest};
use snapshade_core::geometry::PhysicalSize;
use snapshade_core::topology::DisplayTopology;
use snapshade_platform_api::{
    DisplayProvider, OverlayHost, Permission, PlatformError, ScreenCapturer,
};
use snapshade_platform_fake::{FakeCapturer, FakeDisplayProvider, FakeOverlayHost};

const LAYOUT: &str = "retina-plus-1x-right";

fn layout() -> DisplayTopology {
    FakeDisplayProvider::from_layout(LAYOUT)
        .snapshot()
        .expect("fake snapshot never fails")
}

#[test]
fn from_layout_builds_the_fixture_topology() {
    let topo = layout();
    assert_eq!(topo.displays.len(), 2, "two displays in {LAYOUT}");
    assert_eq!(topo.displays[0].id, 1);
    assert!(topo.displays[0].primary, "primary first");
    let external = topo.get(2).expect("display 2 present");
    assert_eq!((external.logical.x, external.logical.y), (1512.0, -229.0));
    assert_eq!(external.physical, PhysicalSize { w: 2560, h: 1440 });
    assert_eq!(external.scale, 1.0);
}

#[test]
#[should_panic(expected = "unknown fixture layout")]
fn unknown_layout_panics() {
    let _ = FakeDisplayProvider::from_layout("no-such-layout");
}

#[test]
fn capture_all_returns_one_filled_frame_per_display_in_id_order() {
    let capturer = FakeCapturer {
        fill: [1, 2, 3, 255],
        fail_display: None,
    };
    let set = capturer
        .capture_all(&layout(), &CaptureRequest::interactive())
        .expect("capture succeeds");
    let ids: Vec<u32> = set.frames.iter().map(|f| f.display_id).collect();
    assert_eq!(ids, vec![1, 2]);
    for frame in &set.frames {
        assert_eq!(
            frame.rgba.len(),
            frame.expected_len(),
            "display {}",
            frame.display_id
        );
        let fill = [1u8, 2, 3, 255];
        assert!(
            frame
                .rgba
                .iter()
                .enumerate()
                .all(|(i, b)| *b == fill[i % 4]),
            "display {} is filled with the fill colour",
            frame.display_id
        );
    }
    let retina = set.get(1).expect("display 1 frame");
    assert_eq!(
        (retina.width, retina.height, retina.scale),
        (3024, 1964, 2.0)
    );
    let external = set.get(2).expect("display 2 frame");
    assert_eq!(
        (external.width, external.height, external.scale),
        (2560, 1440, 1.0)
    );
}

#[test]
fn empty_topology_is_no_displays_error() {
    let capturer = FakeCapturer::default();
    assert_eq!(
        capturer.capture_all(
            &DisplayTopology::new(vec![]),
            &CaptureRequest::interactive()
        ),
        Err(CaptureError::NoDisplays)
    );
}

#[test]
fn scripted_display_failure_is_reported_without_partial_frames() {
    let capturer = FakeCapturer {
        fill: [9, 9, 9, 255],
        fail_display: Some(2),
    };
    let result = capturer.capture_all(&layout(), &CaptureRequest::interactive());
    match result {
        Err(CaptureError::DisplayFailed { display_id, .. }) => assert_eq!(display_id, 2),
        other => panic!("expected DisplayFailed for display 2, got {other:?}"),
    }
}

#[test]
fn permission_is_granted() {
    let capturer = FakeCapturer::default();
    assert_eq!(capturer.permission(), Permission::Granted);
    capturer.request_permission();
    assert_eq!(capturer.permission(), Permission::Granted);
}

#[test]
fn overlay_host_records_calls_in_order() {
    let mut host = FakeOverlayHost::default();
    host.prepare(&layout()).expect("prepare");
    host.show_all().expect("show_all");
    host.hide_all().expect("hide_all");
    assert_eq!(host.calls, vec!["prepare", "show_all", "hide_all"]);
}

#[test]
fn show_all_before_prepare_is_an_os_error() {
    let mut host = FakeOverlayHost::default();
    assert!(
        matches!(host.show_all(), Err(PlatformError::Os(_))),
        "show_all before prepare must fail"
    );
    assert!(host.calls.is_empty(), "a failed call is not recorded");
}
