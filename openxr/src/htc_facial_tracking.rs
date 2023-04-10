use std::{ptr, sync::Arc};

use crate::*;

pub struct FacialTrackerHTC {
    session: Arc<session::SessionInner>,
    handle: sys::FacialTrackerHTC,
    expression_count: usize,
}

impl FacialTrackerHTC {
    #[inline]
    pub fn as_raw(&self) -> sys::FacialTrackerHTC {
        self.handle
    }

    /// Take ownership of an existing facial tracker
    ///
    /// # Safety
    ///
    /// `handle` must be a valid facial tracker handle associated with `session`.
    #[inline]
    pub unsafe fn from_raw<G>(
        session: &Session<G>,
        handle: sys::FacialTrackerHTC,
        expression_count: usize,
    ) -> Self {
        Self {
            handle,
            session: session.inner.clone(),
            expression_count,
        }
    }

    #[inline]
    fn fp(&self) -> &raw::FacialTrackingHTC {
        self.session
            .instance
            .exts()
            .htc_facial_tracking
            .as_ref()
            .expect("Somehow created FacialTrackerHTC without XR_HTC_facial_tracking being enabled")
    }

    #[inline]
    pub fn get_facial_expressions(&self) -> Result<Option<FaceExpressionWeightsHTC>> {
        let mut weights = Vec::with_capacity(self.expression_count);

        let mut facial_expressions = sys::FacialExpressionsHTC {
            ty: sys::FaceExpressionWeightsFB::TYPE,
            next: ptr::null_mut(),
            is_active: sys::FALSE,
            sample_time: Time::from_nanos(0),
            expression_count: self.expression_count as u32,
            expression_weightings: weights.as_mut_ptr(),
        };

        unsafe {
            cvt((self.fp().get_facial_expressions)(
                self.handle,
                &mut facial_expressions,
            ))?;

            if facial_expressions.is_active.into() {
                Ok(Some(FaceExpressionWeightsHTC {
                    weights,
                    sample_time: facial_expressions.sample_time,
                }))
            } else {
                Ok(None)
            }
        }
    }
}

impl<G> Session<G> {
    pub fn create_facial_tracker_htc(
        &self,
        facial_tracking_type: FacialTrackingTypeHTC,
    ) -> Result<FacialTrackerHTC> {
        let fp = self
            .inner
            .instance
            .exts()
            .htc_facial_tracking
            .as_ref()
            .ok_or(sys::Result::ERROR_EXTENSION_NOT_PRESENT)?;

        let mut out = sys::FacialTrackerHTC::NULL;
        let info = sys::FacialTrackerCreateInfoHTC {
            ty: sys::FacialTrackerCreateInfoHTC::TYPE,
            next: ptr::null(),
            facial_tracking_type,
        };
        let handle = unsafe {
            cvt((fp.create_facial_tracker)(self.as_raw(), &info, &mut out))?;
            out
        };
        let expression_count = if facial_tracking_type == FacialTrackingTypeHTC::EYE_DEFAULT {
            sys::FACIAL_EXPRESSION_EYE_COUNT_HTC
        } else {
            sys::FACIAL_EXPRESSION_LIP_COUNT_HTC
        };

        Ok(FacialTrackerHTC {
            session: self.inner.clone(),
            handle,
            expression_count,
        })
    }
}

impl Drop for FacialTrackerHTC {
    fn drop(&mut self) {
        unsafe {
            (self.fp().destroy_facial_tracker)(self.handle);
        }
    }
}

pub struct FaceExpressionWeightsHTC {
    pub weights: Vec<f32>,
    pub sample_time: Time,
}
