use std::{ptr, sync::Arc};

use crate::*;

pub const BODY_JOINT_COUNT_FB: usize = 70;

pub struct BodyTrackerFB {
    pub(crate) session: Arc<session::SessionInner>,
    handle: sys::BodyTrackerFB,
}

impl BodyTrackerFB {
    #[inline]
    pub fn as_raw(&self) -> sys::BodyTrackerFB {
        self.handle
    }

    /// Take ownership of an existing body tracker
    ///
    /// # Safety
    ///
    /// `handle` must be a valid body tracker handle associated with `session`.
    #[inline]
    pub unsafe fn from_raw<G>(session: &Session<G>, handle: sys::BodyTrackerFB) -> Self {
        Self {
            handle,
            session: session.inner.clone(),
        }
    }

    #[inline]
    pub(crate) fn fp(&self) -> &raw::BodyTrackingFB {
        self.session
            .instance
            .exts()
            .fb_body_tracking
            .as_ref()
            .expect("Somehow created BodyTrackingFB without XR_FB_body_tracking being enabled")
    }
}

impl<G> Session<G> {
    pub fn create_body_tracker_fb(&self) -> Result<BodyTrackerFB> {
        let fp = self
            .inner
            .instance
            .exts()
            .fb_body_tracking
            .as_ref()
            .ok_or(sys::Result::ERROR_EXTENSION_NOT_PRESENT)?;

        let mut out = sys::BodyTrackerFB::NULL;
        let info = sys::BodyTrackerCreateInfoFB {
            ty: sys::BodyTrackerCreateInfoFB::TYPE,
            next: ptr::null(),
            body_joint_set: BodyJointSetFB::DEFAULT,
        };
        let handle = unsafe {
            cvt((fp.create_body_tracker)(self.as_raw(), &info, &mut out))?;
            out
        };
        Ok(BodyTrackerFB {
            session: self.inner.clone(),
            handle,
        })
    }
}

impl Drop for BodyTrackerFB {
    fn drop(&mut self) {
        unsafe {
            (self.fp().destroy_body_tracker)(self.handle);
        }
    }
}

/// An array of `BodyJointLocationFB`s, one for each `BodyJointFB`.
///
/// `BodyJointFB`s can be used directly as an index for convenience.
pub type BodyJointFBLocations = [BodyJointLocationFB; BODY_JOINT_COUNT_FB];