ERRORS: exit codes: 0 success; 2 usage; 3 invalid scene or program;
4 device, GPU memory, encoder or setup; 5 filesystem IO.
A stdout consumer closing its pipe early exits successfully. Other stdout write failures
are filesystem IO errors.

If setup cannot open a GPU on Linux, it checks Vulkan ICD driver files and names
libraries that cannot load or expose Vulkan entry points. A GPU visible in
nvidia-smi can still have a broken ICD. Use VK_LOADER_DEBUG=error,warn vork setup
for loader details; see docs/REMOTE-GPU.md for headless NVIDIA driver discovery.
