use std::{ptr, sync::Arc};

use crate::*;

pub struct EyeTrackerSocial {
    session: Arc<session::SessionInner>,
    handle: sys::EyeTrackerFB,
}

impl EyeTrackerSocial {
    #[inline]
    pub fn as_raw(&self) -> sys::EyeTrackerFB {
        self.handle
    }

    /// Take ownership of an existing eye tracker
    ///
    /// # Safety
    ///
    /// `handle` must be a valid eye tracker handle associated with `session`.
    #[inline]
    pub unsafe fn from_raw<G>(session: &Session<G>, handle: sys::EyeTrackerFB) -> Self {
        Self {
            handle,
            session: session.inner.clone(),
        }
    }

    #[inline]
    fn fp(&self) -> &raw::EyeTrackingSocialFB {
        self.session
            .instance
            .exts()
            .fb_eye_tracking_social
            .as_ref()
            .expect(
                "Somehow created EyeTrackerSocial without XR_FB_eye_tracking_social being enabled",
            )
    }

    #[inline]
    pub fn get_eye_gazes(&self, base: &Space, time: Time) -> Result<EyeGazes> {
        // This assert allows this function to be safe.
        assert_eq!(&*self.session as *const session::SessionInner, &*base.session as *const session::SessionInner,
                   "`self` and `base` must have been created, allocated, or retrieved from the same `Session`");

        let gaze_info = sys::EyeGazesInfoFB {
            ty: sys::EyeGazesInfoFB::TYPE,
            next: ptr::null(),
            base_space: base.as_raw(),
            time,
        };

        let mut eye_gazes = sys::EyeGazesFB::out(ptr::null_mut());

        let eye_gazes = unsafe {
            cvt((self.fp().get_eye_gazes)(
                self.handle,
                &gaze_info,
                eye_gazes.as_mut_ptr(),
            ))?;

            eye_gazes.assume_init()
        };

        let left_valid: bool = eye_gazes.gaze[0].is_valid.into();
        let right_valid: bool = eye_gazes.gaze[1].is_valid.into();

        Ok(EyeGazes {
            gaze: [
                left_valid.then(|| EyeGaze {
                    pose: eye_gazes.gaze[0].gaze_pose,
                    confidence: eye_gazes.gaze[0].gaze_confidence,
                }),
                right_valid.then(|| EyeGaze {
                    pose: eye_gazes.gaze[1].gaze_pose,
                    confidence: eye_gazes.gaze[1].gaze_confidence,
                }),
            ],
            time: eye_gazes.time,
        })
    }
}

impl<G> Session<G> {
    pub fn create_eye_tracker_social(&self) -> Result<EyeTrackerSocial> {
        let fp = self
            .inner
            .instance
            .exts()
            .fb_eye_tracking_social
            .as_ref()
            .ok_or(sys::Result::ERROR_EXTENSION_NOT_PRESENT)?;

        let mut out = sys::EyeTrackerFB::NULL;
        let info = sys::EyeTrackerCreateInfoFB {
            ty: sys::EyeTrackerCreateInfoFB::TYPE,
            next: ptr::null(),
        };
        let handle = unsafe {
            cvt((fp.create_eye_tracker)(self.as_raw(), &info, &mut out))?;
            out
        };
        Ok(EyeTrackerSocial {
            session: self.inner.clone(),
            handle,
        })
    }
}

impl Drop for EyeTrackerSocial {
    fn drop(&mut self) {
        unsafe {
            (self.fp().destroy_eye_tracker)(self.handle);
        }
    }
}

pub struct EyeGazes {
    pub gaze: [Option<EyeGaze>; 2],
    pub time: Time,
}

pub struct EyeGaze {
    pub pose: Posef,
    pub confidence: f32,
}
