use std::{mem::MaybeUninit, ptr, sync::Arc};

use crate::*;

pub struct BodyTrackerFB {
    session: Arc<session::SessionInner>,
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
    fn fp(&self) -> &raw::BodyTrackingFB {
        self.session
            .instance
            .exts()
            .fb_body_tracking
            .as_ref()
            .expect("Somehow created BodyTrackerFB without XR_FB_body_tracking being enabled")
    }

    #[inline]
    pub fn locate_body_joints(
        &self,
        space: &Space,
        time: Time,
    ) -> Result<LocateBodyJointsFBResult> {
        let locate_info = sys::BodyJointsLocateInfoFB {
            ty: sys::BodyJointsLocateInfoFB::TYPE,
            next: ptr::null(),
            base_space: space.as_raw(),
            time,
        };

        let mut joint_locations = unsafe {
            MaybeUninit::<[sys::BodyJointLocationFB; BODY_JOINT_COUNT]>::zeroed().assume_init()
        };

        let mut location_data = sys::BodyJointLocationsFB {
            ty: sys::BodyJointLocationsFB::TYPE,
            next: ptr::null_mut(),
            joint_count: BODY_JOINT_COUNT as u32,
            joint_locations: joint_locations.as_mut_ptr() as _,
            confidence: 0.0,
            is_active: false.into(),
            skeleton_changed_count: 0,
            time,
        };

        let result = unsafe {
            (self.fp().locate_body_joints)(self.handle, &locate_info, &mut location_data)
        };

        return match result.into() {
            sys::Result::SUCCESS => Ok(LocateBodyJointsFBResult {
                joint_locations,
                confidence: location_data.confidence,
                is_active: location_data.is_active.into(),
                skeleton_changed_count: location_data.skeleton_changed_count,
                time: location_data.time,
            }),
            e => Err(e.into()),
        };
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

pub struct LocateBodyJointsFBResult {
    pub joint_locations: [BodyJointLocationFB; BODY_JOINT_COUNT],
    pub confidence: f32,
    pub is_active: bool,
    pub skeleton_changed_count: u32,
    pub time: Time,
}

const BODY_JOINT_COUNT: usize = 70;
