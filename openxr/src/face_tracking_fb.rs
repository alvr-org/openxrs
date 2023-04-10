use std::{mem::MaybeUninit, ptr, sync::Arc};

use crate::*;

pub struct FaceTrackerFB {
    session: Arc<session::SessionInner>,
    handle: sys::FaceTrackerFB,
}

impl FaceTrackerFB {
    #[inline]
    pub fn as_raw(&self) -> sys::FaceTrackerFB {
        self.handle
    }

    /// Take ownership of an existing face tracker
    ///
    /// # Safety
    ///
    /// `handle` must be a valid face tracker handle associated with `session`.
    #[inline]
    pub unsafe fn from_raw<G>(session: &Session<G>, handle: sys::FaceTrackerFB) -> Self {
        Self {
            handle,
            session: session.inner.clone(),
        }
    }

    #[inline]
    fn fp(&self) -> &raw::FaceTrackingFB {
        self.session
            .instance
            .exts()
            .fb_face_tracking
            .as_ref()
            .expect("Somehow created FaceTrackerFB without XR_FB_face_tracking being enabled")
    }

    #[inline]
    pub fn get_face_expression_weights(
        &self,
        time: Time,
    ) -> Result<Option<FaceExpressionWeightsFB>> {
        let expression_info = sys::FaceExpressionInfoFB {
            ty: sys::FaceExpressionInfoFB::TYPE,
            next: ptr::null(),
            time,
        };

        let mut weights = MaybeUninit::<[f32; FACE_EXPRESSION_COUNT]>::uninit();
        let mut confidences = [0.0; FACE_CONFIDENCE_COUNT];

        let mut expression_weights = sys::FaceExpressionWeightsFB {
            ty: sys::FaceExpressionWeightsFB::TYPE,
            next: ptr::null_mut(),
            weight_count: FACE_EXPRESSION_COUNT as u32,
            weights: weights.as_mut_ptr() as _,
            confidence_count: FACE_CONFIDENCE_COUNT as u32,
            confidences: confidences.as_mut_ptr() as _,
            status: sys::FaceExpressionStatusFB::default(),
            time,
        };

        unsafe {
            cvt((self.fp().get_face_expression_weights)(
                self.handle,
                &expression_info,
                &mut expression_weights,
            ))?;

            if expression_weights.status.is_valid.into() {
                Ok(Some(FaceExpressionWeightsFB {
                    weights: weights.assume_init(),
                    lower_face_confidence: confidences[0],
                    upper_face_confidence: confidences[1],
                    is_eye_following_blendshapes_valid: expression_weights
                        .status
                        .is_eye_following_blendshapes_valid
                        .into(),
                    time,
                }))
            } else {
                Ok(None)
            }
        }
    }
}

impl<G> Session<G> {
    pub fn create_face_tracker_fb(&self) -> Result<FaceTrackerFB> {
        let fp = self
            .inner
            .instance
            .exts()
            .fb_face_tracking
            .as_ref()
            .ok_or(sys::Result::ERROR_EXTENSION_NOT_PRESENT)?;

        let mut out = sys::FaceTrackerFB::NULL;
        let info = sys::FaceTrackerCreateInfoFB {
            ty: sys::FaceTrackerCreateInfoFB::TYPE,
            next: ptr::null(),
            face_expression_set: FaceExpressionSetFB::DEFAULT,
        };
        let handle = unsafe {
            cvt((fp.create_face_tracker)(self.as_raw(), &info, &mut out))?;
            out
        };
        Ok(FaceTrackerFB {
            session: self.inner.clone(),
            handle,
        })
    }
}

impl Drop for FaceTrackerFB {
    fn drop(&mut self) {
        unsafe {
            (self.fp().destroy_face_tracker)(self.handle);
        }
    }
}

pub struct FaceExpressionWeightsFB {
    pub weights: [f32; FACE_EXPRESSION_COUNT],
    pub lower_face_confidence: f32,
    pub upper_face_confidence: f32,
    pub is_eye_following_blendshapes_valid: bool,
    pub time: Time,
}

const FACE_EXPRESSION_COUNT: usize = 63;
const FACE_CONFIDENCE_COUNT: usize = 2;
