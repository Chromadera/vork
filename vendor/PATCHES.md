# Local changes to vendored crates

vork vendors patched copies of these crates; the edits are in the vendored source and git history.
Each row says why a patch exists and when it can be removed.

| Patch contract | Packages / source | Regression / removal condition |
| --- | --- | --- |
| No-op passthrough shaders and mock TLAS descriptors | wgpu-core device/resource.rs; wgpu-hal noop/mod.rs | GPU-free `scripts/prepare-shaders` and installed archive L4 acceptance; remove when upstream accepts native descriptors on no-op |
| Actual Vulkan buffer memory properties | wgpu-hal, vulkan/mod.rs | Renderer `buffer_heap_tests`; remove when upstream exposes allocator-selected buffer memory metadata |
| Passthrough MSL invariance | wgpu-hal, metal/device.rs | `scripts/check-metal --gpu`; remove when upstream preserves invariance for passthrough MSL |
| Safe-math Naga MSL (WGSL modules) | wgpu-hal, metal/device.rs | `compute::tests::ray_techniques_match_authored_compute_instances` on Metal; remove when upstream compiles Naga MSL without fast math |
| Safe-math passthrough MSL uses precise library functions | wgpu-hal, metal/device.rs | `pipelines::tests::frame_pixels_do_not_depend_on_variant_compile_timing` on Metal; remove when upstream exposes per-module MTLCompileOptions math settings |
| All Metal completions checked; failed teardown quarantined | wgpu-hal, metal/{completion,mod,device}.rs; wgpu-core device/queue.rs | `metal::completion::tests`, gpu_memory tests; remove when upstream checks every buffer before fence success and reports failed drains without destroying in-flight resources |
| Native resource-limit error | wgpu-hal DeviceError::ResourceLimit and Metal pipeline layout; wgpu-core DeviceError conversion without device loss | `medium::tests::native_buffer_slot_limit_is_a_validation_error_not_device_loss` on Metal; remove when upstream reports per-stage Metal slot exhaustion as a validation error |
| Shadow device-loss reason extension | wgpu-hal DeviceError::LostWithReason and Metal wait; wgpu-core device error conversion, loss delivery and queue teardown | `scripts/check-metal-contracts`, gpu_memory tests; remove when upstream carries native failed-buffer error and command label to the device-loss callback |
| Declarative Metal slot consumption | wgpu-types binding.rs; wgpu-hal metal/device.rs | `native::tests` plus native reflection; remove when upstream exposes an equivalent shared representation contract |
| Public poll device loss | wgpu, wgpu-types | `gpu_memory::tests`; remove when upstream returns loss from poll |
| Duplicate intersection setter | wgpu-hal, metal/conv.rs | `rt::tests`; remove when upstream calls the setter |
| Metal buffer arrays / offsets / residency | wgpu-hal, metal/{adapter,device}.rs | `scripts/check-metal-contracts`, native_argument_prefix_tail_execution; remove when upstream supports Tier-2 buffer arrays |
| Declared argument capacity / initialized tail | wgpu-hal, metal/device.rs | `scripts/check-metal-contracts`, argument_buffer_tests; remove when upstream allocates declared capacity and initializes the tail |
| Flexible MSL argument tables use pipeline capacities | wgpu-hal, metal/{mod,device}.rs | `scripts/check-metal-contracts` small render and descriptor-capacity tests in both API-validation modes, argument_buffer_tests; remove when upstream specializes flexible passthrough tables from the stage's binding layout before Metal compilation |
| Counter resolve before reuse | wgpu-hal, metal/command.rs | `frame::profiling_tests` on Metal; remove when upstream orders both counter dependencies |
| Tracked GPU-written TLAS descriptors | wgpu API; wgpu-core commands/trace; wgpu-hal native descriptor layout | `gpu_tlas::tests`, native descriptor build/query/rebuild contracts, core range tests; remove when upstream provides equivalent tracked buffers, dependency retention and backend ABI authority |
| AS build/query/rebuild ordering | wgpu-hal, metal/{adapter,mod,command}.rs | `scripts/check-metal-contracts`, both cross-buffer AS tests; remove only with equivalent native dependency lowering; never restore the no-op AS barrier |
| Bounded pass segments | wgpu API command_encoder.rs, device.rs | `command_finish::tests::recordings_with_more_passes_than_native_admission_complete_in_order` on Metal; remove when upstream Metal no longer needs a native command buffer per pass before submission |
| Typed recording boundaries / merged Vulkan submission | wgpu API; wgpu-core commands/trace; wgpu-hal Vulkan queue | `command_finish::tests` and repeated L4 clips with sync validation; retain core barriers and one native submit per HAL call, never per-cut relay chains |
| Vulkan read-only depth/stencil stores | wgpu-hal Vulkan adapter/device | Read-only depth cut regression and sync validation; remove when upstream uses STORE_OP_NONE with matching capability admission |
| Omitted HAL examples and example dependencies | wgpu-hal Cargo manifests | `cargo check --workspace --all-targets --locked`; retain while vendoring library-only |
| DWA chunk count admission | exr compression/dwa | vendored DWA tests and CLI malformed-image tests; remove when upstream bounds counts before allocation |
| Optional Basis statistics / UASTC hint work | basisu_c_sys encoder/wrapper | compressed_textures tests; remove when upstream offers equivalent opt-outs preserving final ASTC/BC7 blocks |

Upstream trackers: [wgpu](https://github.com/gfx-rs/wgpu/issues),
[exrs](https://github.com/johannesvollmer/exrs/issues),
[basisu_c_sys](https://github.com/beicause/basisu_c_sys/issues).
These are local extensions/fixes; no unverified upstream issue numbers are implied.
Native tests require actual hardware; a successful reapplication proves source
reproducibility, not GPU correctness. Upgrades must pass both checks.
