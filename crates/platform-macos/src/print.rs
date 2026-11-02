//! `NSPrintOperation` with the image scaled to fit the printable page (EXPORT-05;
//! PITFALLS Pitfall 20): an `NSImageView` the size of the imageable page bounds with
//! proportional down-scaling; landscape when the image is wider than tall.

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSImageScaling, NSImageView, NSPaperOrientation, NSPrintInfo, NSPrintOperation,
    NSPrintingPaginationMode,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use snapshade_core::capture::CaptureFrame;
use snapshade_platform_api::{PlatformError, Printer};

use crate::bitmap::ns_image;

/// `Printer` backed by AppKit's print panel. Runs the panel modally on the main thread
/// and returns once it closes; the result is `Ok` whether the user printed or cancelled.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacPrinter;

impl Printer for MacPrinter {
    fn print_image(&self, frame: &CaptureFrame) -> Result<(), PlatformError> {
        let mtm = MainThreadMarker::new().ok_or(PlatformError::MainThreadRequired)?;
        let image = ns_image(frame, mtm)?;

        let info = NSPrintInfo::sharedPrintInfo();
        info.setOrientation(if frame.width > frame.height {
            NSPaperOrientation::Landscape
        } else {
            NSPaperOrientation::Portrait
        });
        info.setHorizontalPagination(NSPrintingPaginationMode::Fit);
        info.setVerticalPagination(NSPrintingPaginationMode::Fit);
        info.setHorizontallyCentered(true);
        info.setVerticallyCentered(true);

        // The view is exactly the imageable area; the image view scales the bitmap down
        // to fit it, preserving aspect ratio, so nothing clips and nothing prints 1 px = 1 pt.
        let page = info.imageablePageBounds();
        let view = NSImageView::imageViewWithImage(&image, mtm);
        view.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(page.size.width, page.size.height),
        ));
        view.setImageScaling(NSImageScaling::ScaleProportionallyDown);

        let op = NSPrintOperation::printOperationWithView_printInfo(&view, &info);
        op.setShowsPrintPanel(true);
        // Modal: blocks this (main) thread inside AppKit's own run loop until the panel
        // closes; the webviews keep painting because that loop still pumps events.
        let _printed = op.runOperation();
        Ok(())
    }
}
