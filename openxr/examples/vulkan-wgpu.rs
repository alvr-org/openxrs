//! Illustrates rendering using Vulkan with multiview. Supports any Vulkan 1.1 capable environment.
//!
//! Renders a smooth gradient across the entire view, with different colors per eye.
//!
//! This example uses minimal abstraction for clarity. Real-world code should encapsulate and
//! largely decouple its Vulkan and OpenXR components and handle errors gracefully.

use std::{
    io::Cursor,
    iter,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use ash::{
    version::{DeviceV1_0, EntryV1_0, InstanceV1_0},
    vk::{self, Handle},
};
use gfx_backend_vulkan as back;
use gfx_hal::{command, device, format, image, memory, pass, pool, prelude::*, pso};
use openxr as xr;

#[allow(clippy::field_reassign_with_default)] // False positive, might be fixed 1.51
fn main() {
    // Handle interrupts gracefully
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::Relaxed);
    })
    .expect("setting Ctrl-C handler");

    #[cfg(feature = "static")]
    let entry = xr::Entry::linked();
    #[cfg(not(feature = "static"))]
    let entry = xr::Entry::load()
        .expect("couldn't find the OpenXR loader; try enabling the \"static\" feature");

    // OpenXR will fail to initialize if we ask for an extension that OpenXR can't provide! So we
    // need to check all our extensions before initializing OpenXR with them. Note that even if the
    // extension is present, it's still possible you may not be able to use it. For example: the
    // hand tracking extension may be present, but the hand sensor might not be plugged in or turned
    // on. There are often additional checks that should be made before using certain features!
    let available_extensions = entry.enumerate_extensions().unwrap();

    // If a required extension isn't present, you want to ditch out here! It's possible something
    // like your rendering API might not be provided by the active runtime. APIs like OpenGL don't
    // have universal support.
    assert!(available_extensions.khr_vulkan_enable2);

    // Initialize OpenXR with the extensions we've found!
    let mut enabled_extensions = xr::ExtensionSet::default();
    enabled_extensions.khr_vulkan_enable2 = true;
    let xr_instance = entry
        .create_instance(
            &xr::ApplicationInfo {
                application_name: "openxrs example",
                application_version: 0,
                engine_name: "openxrs example",
                engine_version: 0,
            },
            &enabled_extensions,
            &[],
        )
        .unwrap();
    let instance_props = xr_instance.properties().unwrap();
    println!(
        "loaded OpenXR runtime: {} {}",
        instance_props.runtime_name, instance_props.runtime_version
    );

    // Request a form factor from the device (HMD, Handheld, etc.)
    let system = xr_instance
        .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
        .unwrap();

    // Check what blend mode is valid for this device (opaque vs transparent displays). We'll just
    // take the first one available!
    let environment_blend_mode = xr_instance
        .enumerate_environment_blend_modes(system, VIEW_TYPE)
        .unwrap()[0];

    // OpenXR wants to ensure apps are using the correct graphics card and Vulkan features and
    // extensions, so the instance and device MUST be set up before Instance::create_session.

    let vk_target_version = vk::make_version(1, 1, 0); // Vulkan 1.1 guarantees multiview support
    let vk_target_version_xr = xr::Version::new(1, 1, 0);

    let reqs = xr_instance
        .graphics_requirements::<xr::Vulkan>(system)
        .unwrap();
    if vk_target_version_xr < reqs.min_api_version_supported
        || vk_target_version_xr.major() > reqs.max_api_version_supported.major()
    {
        panic!(
            "OpenXR runtime requires Vulkan version > {}, < {}.0.0",
            reqs.min_api_version_supported,
            reqs.max_api_version_supported.major() + 1
        );
    }

    let vk_entry = unsafe { ash::Entry::new().unwrap() };

    let vk_app_info = vk::ApplicationInfo::builder()
        .application_version(0)
        .engine_version(0)
        .api_version(vk_target_version);

    let extensions =
        back::Instance::required_extensions(&vk_entry, vk_target_version.into()).unwrap();
    let extensions_ptrs = extensions.iter().map(|&s| s.as_ptr()).collect::<Vec<_>>();

    let layers = back::Instance::required_layers(&vk_entry).unwrap();
    let layers_ptrs = layers.iter().map(|&s| s.as_ptr()).collect::<Vec<_>>();

    let vk_instance = unsafe {
        let vk_instance = xr_instance
            .create_vulkan_instance(
                system,
                std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                &vk::InstanceCreateInfo::builder()
                    .application_info(&vk_app_info)
                    .enabled_extension_names(&extensions_ptrs)
                    .enabled_layer_names(&layers_ptrs) as *const _ as *const _,
            )
            .expect("XR error creating Vulkan instance")
            .map_err(vk::Result::from_raw)
            .expect("Vulkan error creating Vulkan instance");
        ash::Instance::load(
            vk_entry.static_fn(),
            vk::Instance::from_raw(vk_instance as _),
        )
    };
    let vk_instance =
        drop_guard::DropGuard::new(vk_instance, |inst| unsafe { inst.destroy_instance(None) });

    let gfx_instance = unsafe {
        back::Instance::from_raw(
            vk_entry.clone(),
            vk_instance.clone(),
            vk_target_version.into(),
            extensions,
        )
        .unwrap()
    };

    let vk_physical_device = vk::PhysicalDevice::from_raw(
        xr_instance
            .vulkan_graphics_device(system, vk_instance.handle().as_raw() as _)
            .unwrap() as _,
    );

    let gfx_adapter = unsafe { gfx_instance.adapter_from_raw(vk_physical_device) };

    unsafe {
        let vk_device_properties = vk_instance.get_physical_device_properties(vk_physical_device);
        if vk_device_properties.api_version < vk_target_version {
            vk_instance.destroy_instance(None);
            panic!("Vulkan phyiscal device doesn't support version 1.1");
        }
    }

    let queue_family = gfx_adapter
        .queue_families
        .iter()
        .find(|family| family.queue_type().supports_graphics())
        .expect("Vulkan device has no graphics queue");

    let vk_device = unsafe {
        let vk_device = xr_instance
            .create_vulkan_device(
                system,
                std::mem::transmute(vk_entry.static_fn().get_instance_proc_addr),
                vk_physical_device.as_raw() as _,
                &vk::DeviceCreateInfo::builder().queue_create_infos(&[
                    vk::DeviceQueueCreateInfo::builder()
                        .queue_family_index(queue_family.id().0 as _)
                        .queue_priorities(&[1.0])
                        .build(),
                ]) as *const _ as *const _,
            )
            .expect("XR error creating Vulkan device")
            .map_err(vk::Result::from_raw)
            .expect("Vulkan error creating Vulkan device");
        ash::Device::load(vk_instance.fp_v1_0(), vk::Device::from_raw(vk_device as _))
    };
    let vk_device =
        drop_guard::DropGuard::new(vk_device, |dev| unsafe { dev.destroy_device(None) });

    // the arguments should mirror the ones for the creation of vk_device
    let mut gpu = unsafe {
        gfx_adapter
            .physical_device
            .gpu_from_raw(
                vk_device.clone(),
                &[(queue_family, &[1.0])],
                gfx_hal::Features::empty(), // add multiview when supported
            )
            .unwrap()
    };

    let wgpu_instance =
        wgpu::Instance::from_gfx(wgpu_core::instance::GfxInstance::Vulkan(gfx_instance));

    let queue = &mut gpu.queue_groups.pop().unwrap().queues[0];

    let render_pass = unsafe {
        gpu.device
            .create_render_pass(
                iter::once(pass::Attachment {
                    format: Some(COLOR_FORMAT),
                    samples: 1,
                    ops: pass::AttachmentOps::new(
                        pass::AttachmentLoadOp::Clear,
                        pass::AttachmentStoreOp::Store,
                    ),
                    stencil_ops: pass::AttachmentOps::DONT_CARE,
                    layouts: image::Layout::Undefined..image::Layout::ColorAttachmentOptimal,
                }),
                // Note: gfx-hal supports only GRAPHICS bind point
                iter::once(pass::SubpassDesc {
                    colors: &[(0, image::Layout::ColorAttachmentOptimal)],
                    depth_stencil: None,
                    inputs: &[],
                    resolves: &[],
                    preserves: &[],
                }),
                iter::once(pass::SubpassDependency {
                    passes: None..Some(0), // None is SUBPASS_EXTERNAL
                    stages: pso::PipelineStage::COLOR_ATTACHMENT_OUTPUT
                        ..pso::PipelineStage::COLOR_ATTACHMENT_OUTPUT,
                    accesses: image::Access::empty()..image::Access::COLOR_ATTACHMENT_WRITE,
                    flags: memory::Dependencies::empty(),
                }),
            ) // specify multiview when supported
            .unwrap()
    };

    let vert =
        gfx_auxil::read_spirv(Cursor::new(&include_bytes!("fullscreen.vert.spv")[..])).unwrap();
    let frag = gfx_auxil::read_spirv(Cursor::new(
        &include_bytes!("debug_pattern_single_view.frag.spv")[..],
    ))
    .unwrap();
    let vert = unsafe { gpu.device.create_shader_module(&vert).unwrap() };
    let frag = unsafe { gpu.device.create_shader_module(&frag).unwrap() };

    let pipeline_layout = unsafe {
        gpu.device
            .create_pipeline_layout(iter::empty(), iter::empty())
            .unwrap()
    };

    let pipeline = unsafe {
        gpu.device
            .create_graphics_pipeline(
                &pso::GraphicsPipelineDesc {
                    primitive_assembler: pso::PrimitiveAssemblerDesc::Vertex {
                        buffers: &[],
                        attributes: &[],
                        input_assembler: pso::InputAssemblerDesc {
                            primitive: pso::Primitive::TriangleList,
                            with_adjacency: false,
                            restart_index: None,
                        },
                        vertex: pso::EntryPoint {
                            entry: "main",
                            module: &vert,
                            specialization: pso::Specialization::EMPTY,
                        },
                        tessellation: None,
                        geometry: None,
                    },
                    fragment: Some(pso::EntryPoint {
                        entry: "main",
                        module: &frag,
                        specialization: pso::Specialization::EMPTY,
                    }),
                    // viewport_state -> gfx-hal always has 1 viewport and 1 scissor
                    rasterizer: pso::Rasterizer::FILL,
                    multisampling: None,
                    depth_stencil: pso::DepthStencilDesc {
                        depth: None,
                        depth_bounds: false,
                        stencil: None,
                    },
                    blender: pso::BlendDesc {
                        logic_op: None,
                        targets: vec![pso::ColorBlendDesc {
                            mask: pso::ColorMask::COLOR,
                            blend: Some(pso::BlendState::ADD),
                        }],
                    },
                    // Note: opposite of dynamic_state
                    baked_states: pso::BakedStates {
                        viewport: None,
                        scissor: None,
                        blend_constants: None,
                        depth_bounds: None,
                    },
                    layout: &pipeline_layout,
                    subpass: pass::Subpass {
                        index: 0,
                        main_pass: &render_pass,
                    },
                    // Other:
                    label: None,
                    flags: pso::PipelineCreationFlags::empty(),
                    parent: pso::BasePipeline::None,
                },
                None,
            )
            .unwrap()
    };

    unsafe {
        gpu.device.destroy_shader_module(vert);
        gpu.device.destroy_shader_module(frag);
    }

    // A session represents this application's desire to display things! This is where we hook
    // up our graphics API. This does not start the session; for that, you'll need a call to
    // Session::begin, which we do in 'main_loop below.
    let (session, mut frame_wait, mut frame_stream) = unsafe {
        xr_instance
            .create_session::<xr::Vulkan>(
                system,
                &xr::vulkan::SessionCreateInfo {
                    instance: vk_instance.handle().as_raw() as _,
                    physical_device: vk_physical_device.as_raw() as _,
                    device: vk_device.handle().as_raw() as _,
                    queue_family_index: queue_family.id().0 as _,
                    queue_index: 0,
                },
            )
            .unwrap()
    };

    // Create an action set to encapsulate our actions
    let action_set = xr_instance
        .create_action_set("input", "input pose information", 0)
        .unwrap();

    let right_action = action_set
        .create_action::<xr::Posef>("right_hand", "Right Hand Controller", &[])
        .unwrap();
    let left_action = action_set
        .create_action::<xr::Posef>("left_hand", "Left Hand Controller", &[])
        .unwrap();

    // Bind our actions to input devices using the given profile
    // If you want to access inputs specific to a particular device you may specify a different
    // interaction profile
    xr_instance
        .suggest_interaction_profile_bindings(
            xr_instance
                .string_to_path("/interaction_profiles/khr/simple_controller")
                .unwrap(),
            &[
                xr::Binding::new(
                    &right_action,
                    xr_instance
                        .string_to_path("/user/hand/right/input/grip/pose")
                        .unwrap(),
                ),
                xr::Binding::new(
                    &left_action,
                    xr_instance
                        .string_to_path("/user/hand/left/input/grip/pose")
                        .unwrap(),
                ),
            ],
        )
        .unwrap();

    // Attach the action set to the session
    session.attach_action_sets(&[&action_set]).unwrap();

    // Create an action space for each device we want to locate
    let right_space = right_action
        .create_space(session.clone(), xr::Path::NULL, xr::Posef::IDENTITY)
        .unwrap();
    let left_space = left_action
        .create_space(session.clone(), xr::Path::NULL, xr::Posef::IDENTITY)
        .unwrap();

    // OpenXR uses a couple different types of reference frames for positioning content; we need
    // to choose one for displaying our content! STAGE would be relative to the center of your
    // guardian system's bounds, and LOCAL would be relative to your device's starting location.
    let stage = session
        .create_reference_space(xr::ReferenceSpaceType::STAGE, xr::Posef::IDENTITY)
        .unwrap();

    let mut cmd_pool = unsafe {
        gpu.device
            .create_command_pool(
                queue_family.id(),
                pool::CommandPoolCreateFlags::RESET_INDIVIDUAL
                    | pool::CommandPoolCreateFlags::TRANSIENT,
            )
            .unwrap()
    };
    let mut cmds = vec![];
    unsafe { cmd_pool.allocate(PIPELINE_DEPTH, command::Level::Primary, &mut cmds) };
    let mut fences = (0..PIPELINE_DEPTH)
        .map(|_| gpu.device.create_fence(true).unwrap())
        .collect::<Vec<_>>();

    // Main loop
    let mut swapchain = None;
    let mut event_storage = xr::EventDataBuffer::new();
    let mut session_running = false;
    // Index of the current frame, wrapped by PIPELINE_DEPTH. Not to be confused with the
    // swapchain image index.
    let mut frame = 0;
    'main_loop: loop {
        if !running.load(Ordering::Relaxed) {
            println!("requesting exit");
            // The OpenXR runtime may want to perform a smooth transition between scenes, so we
            // can't necessarily exit instantly. Instead, we must notify the runtime of our
            // intent and wait for it to tell us when we're actually done.
            match session.request_exit() {
                Ok(()) => {}
                Err(xr::sys::Result::ERROR_SESSION_NOT_RUNNING) => break,
                Err(e) => panic!("{}", e),
            }
        }

        while let Some(event) = xr_instance.poll_event(&mut event_storage).unwrap() {
            use xr::Event::*;
            match event {
                SessionStateChanged(e) => {
                    // Session state change is where we can begin and end sessions, as well as
                    // find quit messages!
                    println!("entered state {:?}", e.state());
                    match e.state() {
                        xr::SessionState::READY => {
                            session.begin(VIEW_TYPE).unwrap();
                            session_running = true;
                        }
                        xr::SessionState::STOPPING => {
                            session.end().unwrap();
                            session_running = false;
                        }
                        xr::SessionState::EXITING | xr::SessionState::LOSS_PENDING => {
                            break 'main_loop;
                        }
                        _ => {}
                    }
                }
                InstanceLossPending(_) => {
                    break 'main_loop;
                }
                EventsLost(e) => {
                    println!("lost {} events", e.lost_event_count());
                }
                _ => {}
            }
        }

        if !session_running {
            // Don't grind up the CPU
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }

        // Block until the previous frame is finished displaying, and is ready for another one.
        // Also returns a prediction of when the next frame will be displayed, for use with
        // predicting locations of controllers, viewpoints, etc.
        let xr_frame_state = frame_wait.wait().unwrap();
        // Must be called before any rendering is done!
        frame_stream.begin().unwrap();

        if !xr_frame_state.should_render {
            frame_stream
                .end(
                    xr_frame_state.predicted_display_time,
                    environment_blend_mode,
                    &[],
                )
                .unwrap();
            continue;
        }

        let swapchain = swapchain.get_or_insert_with(|| {
            // Now we need to find all the viewpoints we need to take care of! This is a
            // property of the view configuration type; in this example we use PRIMARY_STEREO,
            // so we should have 2 viewpoints.
            //
            // Because we are using multiview in this example, we require that all view
            // dimensions are identical.
            let views = xr_instance
                .enumerate_view_configuration_views(system, VIEW_TYPE)
                .unwrap();
            assert_eq!(views.len(), 2);
            assert_eq!(views[0], views[1]);

            // Create a swapchain for the viewpoints! A swapchain is a set of texture buffers
            // used for displaying to screen, typically this is a backbuffer and a front buffer,
            // one for rendering data to, and one for displaying on-screen.
            let resolution = vk::Extent2D {
                width: views[0].recommended_image_rect_width,
                height: views[0].recommended_image_rect_height,
            };
            let handle = session
                .create_swapchain(&xr::SwapchainCreateInfo {
                    create_flags: xr::SwapchainCreateFlags::EMPTY,
                    usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                        | xr::SwapchainUsageFlags::SAMPLED,
                    format: VK_COLOR_FORMAT.as_raw() as _,
                    // The Vulkan graphics pipeline we create is not set up for multisampling,
                    // so we hardcode this to 1. If we used a proper multisampling setup, we
                    // could set this to `views[0].recommended_swapchain_sample_count`.
                    sample_count: 1,
                    width: resolution.width,
                    height: resolution.height,
                    face_count: 1,
                    array_size: 1,
                    mip_count: 1,
                })
                .unwrap();

            // We'll want to track our own information about the swapchain, so we can draw stuff
            // onto it! We'll also create a buffer for each generated texture here as well.
            let images = handle.enumerate_images().unwrap();
            Swapchain {
                handle,
                resolution,
                buffers: images
                    .into_iter()
                    .map(|color_image| {
                        let color_image = vk::Image::from_raw(color_image);
                        let color = unsafe {
                            gpu.device
                                .image_view_from_raw(
                                    color_image,
                                    vk::ImageViewType::TYPE_2D_ARRAY,
                                    COLOR_FORMAT,
                                    format::Swizzle::NO,
                                    image::Usage::COLOR_ATTACHMENT | image::Usage::SAMPLED,
                                    image::SubresourceRange {
                                        aspects: format::Aspects::COLOR,
                                        level_start: 0,
                                        level_count: Some(1),
                                        layer_start: 0,
                                        layer_count: Some(1),
                                    },
                                )
                                .unwrap()
                        };
                        let framebuffer = unsafe {
                            gpu.device
                                .create_framebuffer(
                                    &render_pass,
                                    iter::once(image::FramebufferAttachment {
                                        usage: image::Usage::COLOR_ATTACHMENT,
                                        view_caps: image::ViewCapabilities::empty(),
                                        format: COLOR_FORMAT,
                                    }),
                                    image::Extent {
                                        width: resolution.width,
                                        height: resolution.height,
                                        depth: 1,
                                    },
                                )
                                .unwrap()
                        };
                        Framebuffer { framebuffer, color }
                    })
                    .collect(),
            }
        });

        // We need to ask which swapchain image to use for rendering! Which one will we get?
        // Who knows! It's up to the runtime to decide.
        let image_index = swapchain.handle.acquire_image().unwrap();

        // Wait until the image is available to render to. The compositor could still be
        // reading from it.
        swapchain.handle.wait_image(xr::Duration::INFINITE).unwrap();

        let cmd = &mut cmds[frame];
        let fence = &mut fences[frame as usize];
        let buffer = &swapchain.buffers[image_index as usize];
        unsafe {
            // Ensure the last use of this frame's resources is 100% done
            gpu.device.wait_for_fence(fence, u64::MAX).unwrap();
            gpu.device.reset_fence(fence).unwrap();

            cmd.begin_primary(command::CommandBufferFlags::ONE_TIME_SUBMIT);
            cmd.begin_render_pass(
                &render_pass,
                &buffer.framebuffer,
                pso::Rect {
                    x: 0,
                    y: 0,
                    w: swapchain.resolution.width as i16,
                    h: swapchain.resolution.height as i16,
                },
                iter::once(command::RenderAttachmentInfo {
                    image_view: &buffer.color,
                    clear_value: command::ClearValue {
                        color: command::ClearColor {
                            float32: [0.0, 0.0, 0.0, 1.0],
                        },
                    },
                }),
                command::SubpassContents::Inline,
            );
        }

        let viewports = iter::once(pso::Viewport {
            rect: pso::Rect {
                x: 0,
                y: 0,
                w: swapchain.resolution.width as i16,
                h: swapchain.resolution.height as i16,
            },
            depth: 0.0..1.0,
        });
        let scissors = iter::once(pso::Rect {
            x: 0,
            y: 0,
            w: swapchain.resolution.width as i16,
            h: swapchain.resolution.height as i16,
        });
        unsafe {
            cmd.set_viewports(0, viewports);
            cmd.set_scissors(0, scissors);
        }

        // Draw the scene. Multiview means we only need to do this once, and the GPU will
        // automatically broadcast operations to all views. Shaders can use `gl_ViewIndex` to
        // e.g. select the correct view matrix.
        unsafe {
            cmd.bind_graphics_pipeline(&pipeline);
            cmd.draw(0..3, 0..1);

            cmd.end_render_pass();
            cmd.finish();
        }

        session.sync_actions(&[(&action_set).into()]).unwrap();

        // Find where our controllers are located in the Stage space
        let right_location = right_space
            .locate(&stage, xr_frame_state.predicted_display_time)
            .unwrap();

        let left_location = left_space
            .locate(&stage, xr_frame_state.predicted_display_time)
            .unwrap();

        if left_action.is_active(&session, xr::Path::NULL).unwrap() {
            print!(
                "Left Hand: ({:0<12},{:0<12},{:0<12}), ",
                left_location.pose.position.x,
                left_location.pose.position.y,
                left_location.pose.position.z
            );
        }

        if right_action.is_active(&session, xr::Path::NULL).unwrap() {
            print!(
                "Right Hand: ({:0<12},{:0<12},{:0<12})",
                right_location.pose.position.x,
                right_location.pose.position.y,
                right_location.pose.position.z
            );
        }
        println!();

        // Fetch the view transforms. To minimize latency, we intentionally do this *after*
        // recording commands to render the scene, i.e. at the last possible moment before
        // rendering begins in earnest on the GPU. Uniforms dependent on this data can be sent
        // to the GPU just-in-time by writing them to per-frame host-visible memory which the
        // GPU will only read once the command buffer is submitted.
        let (_, views) = session
            .locate_views(VIEW_TYPE, xr_frame_state.predicted_display_time, &stage)
            .unwrap();

        // Submit commands to the GPU, then tell OpenXR we're done with our part.
        unsafe {
            queue.submit(iter::once(&*cmd), iter::empty(), iter::empty(), Some(fence));
        }
        swapchain.handle.release_image().unwrap();

        // Tell OpenXR what to present for this frame
        let rect = xr::Rect2Di {
            offset: xr::Offset2Di { x: 0, y: 0 },
            extent: xr::Extent2Di {
                width: swapchain.resolution.width as _,
                height: swapchain.resolution.height as _,
            },
        };
        frame_stream
            .end(
                xr_frame_state.predicted_display_time,
                environment_blend_mode,
                &[
                    &xr::CompositionLayerProjection::new().space(&stage).views(&[
                        xr::CompositionLayerProjectionView::new()
                            .pose(views[0].pose)
                            .fov(views[0].fov)
                            .sub_image(
                                xr::SwapchainSubImage::new()
                                    .swapchain(&swapchain.handle)
                                    .image_array_index(0)
                                    .image_rect(rect),
                            ),
                        xr::CompositionLayerProjectionView::new()
                            .pose(views[1].pose)
                            .fov(views[1].fov)
                            .sub_image(
                                xr::SwapchainSubImage::new()
                                    .swapchain(&swapchain.handle)
                                    .image_array_index(0)
                                    .image_rect(rect),
                            ),
                    ]),
                ],
            )
            .unwrap();
        frame = (frame + 1) % PIPELINE_DEPTH as usize;
    }

    // OpenXR MUST be allowed to clean up before we destroy Vulkan resources it could touch, so
    // first we must drop all its handles.
    drop((
        session,
        frame_wait,
        frame_stream,
        stage,
        action_set,
        left_space,
        right_space,
        left_action,
        right_action,
    ));

    // Ensure all in-flight frames are finished before destroying resources they might use
    unsafe {
        gpu.device
            .wait_for_fences(fences.iter(), device::WaitFor::All, u64::MAX)
            .unwrap();
        for fence in fences {
            gpu.device.destroy_fence(fence);
        }

        if let Some(swapchain) = swapchain {
            for buffer in swapchain.buffers {
                gpu.device.destroy_framebuffer(buffer.framebuffer);
                gpu.device.destroy_image_view(buffer.color);
            }
        }

        // vk_device.destroy_pipeline(pipeline.0, None);
        gpu.device.destroy_pipeline_layout(pipeline_layout);
        gpu.device.destroy_command_pool(cmd_pool);
        gpu.device.destroy_render_pass(render_pass);
    }

    println!("exiting cleanly");
}

pub const COLOR_FORMAT: format::Format = format::Format::Bgra8Srgb;
pub const VK_COLOR_FORMAT: vk::Format = vk::Format::B8G8R8A8_SRGB;
const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;

struct Swapchain {
    handle: xr::Swapchain<xr::Vulkan>,
    buffers: Vec<Framebuffer>,
    resolution: vk::Extent2D,
}

struct Framebuffer {
    framebuffer: <back::Backend as gfx_hal::Backend>::Framebuffer,
    color: <back::Backend as gfx_hal::Backend>::ImageView,
}

/// Maximum number of frames in flight
const PIPELINE_DEPTH: usize = 2;
