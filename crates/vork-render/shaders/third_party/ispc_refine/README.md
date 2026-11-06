The PCA/least-squares mode-5/6 fitting in ../../bc7_refine.slangh follows the
MIT-licensed ISPCTextureCompressor mode-5/6 encoders (kernel.ispc), Intel Corporation.
Reference revision:79ddbc90334fc31edd438e68ccb0fe99b4e15aab.
https://github.com/GameTechDev/ISPCTextureCompressor/blob/79ddbc90334fc31edd438e68ccb0fe99b4e15aab/ispc_texcomp/kernel.ispc

The Slang implementation keeps the existing mode4/5/6 block unless the new fit
has lower decoded squared error; alpha-varying blocks use SIMD refinement.
