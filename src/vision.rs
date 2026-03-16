//! Vision framework bindings via objc2-vision
//!
//! https://developer.apple.com/documentation/vision?language=objc

pub use objc2_vision::{
    VNImageRequestHandler, VNRecognizeTextRequest, VNRecognizedText, VNRecognizedTextObservation,
    VNRequest,
};

use objc2::rc::Retained;
use objc2::AnyThread;
use objc2_core_graphics::CGImage;
use objc2_foundation::{NSArray, NSError, NSURL};
use objc2_vision::VNImageOption;
use objc2::runtime::AnyObject;

pub fn new_handler_with_cgimage(
    image: &CGImage,
) -> Retained<VNImageRequestHandler> {
    let options = objc2_foundation::NSDictionary::<VNImageOption, AnyObject>::new();
    unsafe {
        VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            image,
            &options,
        )
    }
}

pub fn new_handler_with_url(url: &NSURL) -> Retained<VNImageRequestHandler> {
    let options = objc2_foundation::NSDictionary::<VNImageOption, AnyObject>::new();
    unsafe {
        VNImageRequestHandler::initWithURL_options(VNImageRequestHandler::alloc(), url, &options)
    }
}

pub fn perform_requests(
    handler: &VNImageRequestHandler,
    requests: &NSArray<VNRequest>,
) -> Result<(), Retained<NSError>> {
    handler.performRequests_error(requests)
}
