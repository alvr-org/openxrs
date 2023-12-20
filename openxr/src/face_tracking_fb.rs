use std::{mem::MaybeUninit, ptr, sync::Arc};

use crate::*;

pub struct FaceTracker2FB {
    session: Arc<session::SessionInner>,
    handle: sys::FaceTracker2FB,
}

impl FaceTracker2FB {
    #[inline]
    pub fn as_raw(&self) -> sys::FaceTracker2FB {
        self.handle
    }

    /// Take ownership of an existing face tracker
    ///
    /// # Safety
    ///
    /// `handle` must be a valid face tracker handle associated with `session`.
    #[inline]
    pub unsafe fn from_raw<G>(session: &Session<G>, handle: sys::FaceTracker2FB) -> Self {
        Self {
            handle,
            session: session.inner.clone(),
        }
    }

    #[inline]
    fn fp(&self) -> &raw::FaceTracking2FB {
        self.session
            .instance
            .exts()
            .fb_face_tracking2
            .as_ref()
            .expect("Somehow created FaceTrackerFB without XR_FB_face_tracking2 being enabled")
    }

    #[inline]
    pub fn get_face_expression_weights(
        &self,
        time: Time,
    ) -> Result<Option<FaceExpressionWeights2FB>> {
        let expression_info = sys::FaceExpressionInfo2FB {
            ty: sys::FaceExpressionInfo2FB::TYPE,
            next: ptr::null(),
            time,
        };

        let mut weights = MaybeUninit::<[f32; FACE_EXPRESSION2_COUNT]>::uninit();
        let mut confidences = [0.0; FACE_CONFIDENCE2_COUNT];

        let mut expression_weights = sys::FaceExpressionWeights2FB {
            ty: sys::FaceExpressionWeights2FB::TYPE,
            next: ptr::null_mut(),
            weight_count: FACE_EXPRESSION2_COUNT as u32,
            weights: weights.as_mut_ptr() as _,
            confidence_count: FACE_CONFIDENCE2_COUNT as u32,
            confidences: confidences.as_mut_ptr() as _,
            is_valid: sys::FALSE,
            is_eye_following_blendshapes_valid: sys::FALSE,
            data_source: sys::FaceTrackingDataSource2FB::from_raw(0),
            time,
        };

        unsafe {
            cvt((self.fp().get_face_expression_weights2)(
                self.handle,
                &expression_info,
                &mut expression_weights,
            ))?;

            if expression_weights.is_valid.into() {
                Ok(Some(FaceExpressionWeights2FB {
                    weights: weights.assume_init(),
                    lower_face_confidence: confidences[0],
                    upper_face_confidence: confidences[1],
                    is_eye_following_blendshapes_valid: expression_weights
                        .is_eye_following_blendshapes_valid
                        .into(),
                    data_source: expression_weights.data_source,
                    time,
                }))
            } else {
                Ok(None)
            }
        }
    }
}

impl<G> Session<G> {
    pub fn create_face_tracker2_fb(&self, visual: bool, audio: bool) -> Result<FaceTracker2FB> {
        let fp = self
            .inner
            .instance
            .exts()
            .fb_face_tracking2
            .as_ref()
            .ok_or(sys::Result::ERROR_EXTENSION_NOT_PRESENT)?;

        let mut requested_data_sources = vec![];
        if visual {
            requested_data_sources.push(sys::FaceTrackingDataSource2FB::VISUAL);
        }
        if audio {
            requested_data_sources.push(sys::FaceTrackingDataSource2FB::AUDIO);
        }

        let mut out = sys::FaceTracker2FB::NULL;
        let info = sys::FaceTrackerCreateInfo2FB {
            ty: sys::FaceTrackerCreateInfo2FB::TYPE,
            next: ptr::null(),
            face_expression_set: FaceExpressionSet2FB::DEFAULT,
            requested_data_source_count: requested_data_sources.len() as u32,
            requested_data_sources: requested_data_sources.as_mut_ptr(),
        };
        let handle = unsafe {
            cvt((fp.create_face_tracker2)(self.as_raw(), &info, &mut out))?;
            out
        };
        Ok(FaceTracker2FB {
            session: self.inner.clone(),
            handle,
        })
    }
}

impl Drop for FaceTracker2FB {
    fn drop(&mut self) {
        unsafe {
            (self.fp().destroy_face_tracker2)(self.handle);
        }
    }
}

pub struct FaceExpressionWeights2FB {
    pub weights: [f32; FACE_EXPRESSION2_COUNT],
    pub lower_face_confidence: f32,
    pub upper_face_confidence: f32,
    pub is_eye_following_blendshapes_valid: bool,
    pub data_source: sys::FaceTrackingDataSource2FB,
    pub time: Time,
}

const FACE_EXPRESSION2_COUNT: usize = 70;
const FACE_CONFIDENCE2_COUNT: usize = 2;
