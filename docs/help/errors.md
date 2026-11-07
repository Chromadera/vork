ERRORS: exit codes: 0 success; 2 usage; 3 invalid scene or program;
4 device, GPU memory, encoder or setup; 5 filesystem IO.
A stdout consumer closing its pipe early exits successfully. Other stdout write failures
are filesystem IO errors.
