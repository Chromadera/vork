//! Camera ray and per-contribution detector stages share ordinary Inputs and effects.
use super::*;

#[derive(Debug, Serialize, Deserialize)]
pub struct CameraProgram {
    #[serde(skip)]
    module: std::sync::OnceLock<naga::Module>,
    pub native: String,
    pub include: Vec<PathBuf>,
    pub wgsl: String,
    pub camera_entry: Option<String>,
    pub sensor_entry: Option<String>,
    pub calibration_entry: Option<String>,
    pub inputs: InputLayout,
    pub dependencies: Vec<PathBuf>,
    pub warnings: String,
}
impl Compiler {
    pub fn compile_camera(&self, abi: &Abi, source: &str, origin: &Path, tag: &str) -> Result<CameraProgram> {
        let compiled = self.compile(abi, source, origin, tag, false, |inputs, declarations| {
            ensure!(declarations.contains("camera"),
                "camera material requires CameraRay camera(CameraContext, Inputs) and optional float3 sensor(SensorContext, Inputs)");
            ensure!(inputs.fields.iter().all(|f| !matches!(f.ty, Type::Geometry | Type::Attribute(_)|Type::Aov|Type::FloatGrid|Type::VectorGrid|Type::Image)),
                "camera and sensor inputs must be numeric");
            let decode = inputs.slang_decode();
            let mut wrapper = "[shader(\"compute\")][numthreads(1,1,1)] void vork_entry() {}\n".to_owned();
            if declarations.contains("camera") {
                writeln!(wrapper, "export void vork_camera_{tag}(CameraContext ctx,uint base,out float4 r[3]) {{ {decode} let ray=camera(ctx,inputs); r[0]=float4(ray.origin.x,ray.origin.y,ray.origin.z,ray.wavelength_scale);r[1]=float4(ray.direction,ray.t_min);r[2]=float4(ray.weight.x,ray.weight.y,ray.weight.z,ray.t_max); }}")?;
            }
            if declarations.contains("sensor") {
                writeln!(wrapper, "export void vork_sensor_{tag}(SensorContext ctx,uint base,out float4 r[1]) {{ {decode} r[0]=float4(sensor(ctx,inputs),0.); }}")?;
            }
            if declarations.contains("camera_calibration") {
                writeln!(wrapper, "export void vork_camera_calibration_{tag}(float aspect,float focus,float time,uint base,out float4 r[4]) {{ {decode} let c=camera_calibration(aspect,focus,time,inputs);r[0]=c.screen;r[1]=c.clip;r[2]=c.aperture;r[3]=c.shape; }}")?;
            }
            Ok(wrapper)
        })?;
        Ok(CameraProgram {
            module: std::sync::OnceLock::new(),
            calibration_entry: compiled.declarations.contains("camera_calibration").then(||format!("vork_camera_calibration_{tag}_0_{tag}")),
            camera_entry: compiled.declarations.contains("camera").then(||format!("vork_camera_{tag}_0_{tag}")),
            sensor_entry: compiled.declarations.contains("sensor").then(||format!("vork_sensor_{tag}_0_{tag}")),
            native: compiled.native.replace("export void", "public void"),
            include: include_paths(origin),
            wgsl: compiled.output.wgsl,
            inputs: compiled.inputs,
            dependencies: compiled.output.dependencies,
            warnings: compiled.output.diagnostics,
        })
    }
}
impl CameraProgram {
    fn module(&self, abi: &Abi) -> Result<&naga::Module> {
        if self.module.get().is_none() { let _ = self.module.set(validated_module(&self.wgsl, abi)?); }
        Ok(self.module.get().unwrap())
    }

    pub fn uses_time(&self, abi: &Abi) -> Result<bool> {
        effects::camera_uses_time(self.module(abi)?, self.camera_entry.as_deref(), self.sensor_entry.as_deref(), self.calibration_entry.as_deref())
    }

    /// Whether calibration can expose an aperture at any clock/input value.
    pub fn aperture_possible(&self, abi: &Abi, words: &[Option<u32>], aspect: f32, focus: Option<f32>) -> Result<bool> {
        self.calibration_entry.as_ref().map(|entry|
            effects::camera_aperture_possible(self.module(abi)?,entry,words,aspect,focus)
        ).transpose().map(|v|v.unwrap_or(true))
    }

    pub fn calibration(&self, abi: &Abi, words: &[u32], aspect: f32, focus: f32, time: f32) -> Result<Option<[[f32;4];4]>> {
        self.calibration_entry.as_ref().map(|entry| {
            effects::camera_calibration(self.module(abi)?,entry,words,aspect,focus,time)
        }).transpose()
    }

    pub fn unit_weight(&self, abi: &Abi, words: &[Option<u32>]) -> Result<bool> {
        effects::camera_unit_weight(self.module(abi)?,self.camera_entry.as_deref().context("camera entry missing")?,words)
    }

    /// Unknown/animated input words remain unknown. Only a proven identity
    /// wavelength mapping and wavelength-independent detector retain RGB.
    pub fn needs_spectral(&self, abi: &Abi, words: &[Option<u32>]) -> Result<bool> {
        let module = self.module(abi)?;
        effects::camera_needs_spectral(module, self.camera_entry.as_deref(), self.sensor_entry.as_deref(), words)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires pinned Slang"]
    fn camera_stage_effects() -> Result<()> {
        let abi=Abi {aov_count:0, context:Abi::spec_context(), closure:openpbr_fields()};
        let compiler=Compiler::discover();
        let cases=[
            ("CameraRay camera(CameraContext c,Inputs i) {CameraRay r;r.origin=c.origin+c.time;r.direction=c.forward;return r;}",false),
            ("float3 sensor(SensorContext c,Inputs i) {return c.path_length>i.scale ? 1.:0.;}",false),
            ("float3 sensor(SensorContext c,Inputs i) {return c.wavelength>500. ? 1.:0.;}",true),
            ("float helper(SensorContext c) {return c.time;} float3 sensor(SensorContext c,Inputs i) {return helper(c);}",false),
            ("CameraRay camera(CameraContext c,Inputs i) {CameraRay r;r.origin=c.origin;r.direction=normalize(c.forward+c.right);r.weight=float3(i.scale);return r;}",false),
            ("CameraRay camera(CameraContext c,Inputs i) {CameraRay r;r.origin=c.origin;r.direction=c.forward;r.wavelength_scale=i.scale;return r;}",true),
        ];
        for (body,expected) in cases {
            let ray=if body.contains("CameraRay camera") {""} else {"CameraRay camera(CameraContext c,Inputs i) {CameraRay r;r.origin=c.origin;r.direction=c.forward;r.t_min=0.;r.t_max=1000.;return r;}"};
            let source=format!("struct Inputs {{float scale=2.;}};{body}{ray}");
            let p=compiler.compile_camera(&abi,&source,Path::new("camera-effects.slang"),"cameraeffects")?;
            let words=p.inputs.pack(&BTreeMap::new(),|_,_|unreachable!())?;
            assert_eq!(p.uses_time(&abi)?,body.contains("c.time"),"{body}");
            assert_eq!(p.needs_spectral(&abi,&words.iter().copied().map(Some).collect::<Vec<_>>())?,expected,"{body}");
            if body.contains("wavelength_scale=i.scale") {
                let mut identity=words.iter().copied().map(Some).collect::<Vec<_>>();
                identity[0]=Some(1);identity[p.inputs.fields[0].offset]=Some(1f32.to_bits());
                assert!(!p.needs_spectral(&abi,&identity)?);
                assert!(p.needs_spectral(&abi,&[None])?);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    #[test]
    #[ignore = "requires pinned Slang"]
    fn surface_spectral_effects_follow_outputs_and_bound_inputs() -> Result<()> {
        let abi=Abi {aov_count:0,context:Abi::spec_context(),closure:openpbr_fields()};
        for (body,expected) in [
            ("c.base_color=float3(.5);",false),
            ("float unused=ctx.wavelength;c.base_color=float3(.5);",false),
            ("c.base_color=float3(ctx.wavelength/780.);",true),
            ("c.transmission_dispersion_scale=1.;",false),
            ("c.transmission_weight=1.;c.transmission_dispersion_scale=i.scale;",true),
        ] {
            let source=format!("struct Inputs {{float scale=1.;}}; Closure surface(ShadingContext ctx,Inputs i) {{Closure c;{body}return c;}}");
            let p=Compiler::discover().compile_surface(&abi,&source,Path::new("spectral-effects.slang"),"spectraleffects")?;
            assert_eq!(p.needs_spectral(&abi,&p.inputs.effect_words(&BTreeMap::new(),&BTreeSet::new())?)?,expected,"{body}");
            if body.contains("i.scale") {
                let values=serde_json::from_value(serde_json::json!({"scale":0}))?;
                assert!(!p.needs_spectral(&abi,&p.inputs.effect_words(&values,&BTreeSet::new())?)?);
                assert!(p.needs_spectral(&abi,&p.inputs.effect_words(&values,&["scale".into()].into())?)?);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod wavelength_tests {
    use super::*;
    #[test]
    #[ignore = "requires pinned Slang"]
    fn medium_and_emitter_wavelength_dependencies() -> Result<()> {
        let abi=Abi {aov_count:0,context:Abi::spec_context(),closure:openpbr_fields()};
        for (expression,expected) in [("c.time",false),("c.wavelength",true)] {
            let source=format!("struct Inputs {{}}; MediumCoefficients medium(MediumContext c,Inputs i) {{MediumCoefficients m;m.scattering=float3({expression});return m;}}");
            let p=Compiler::discover().compile_medium(&abi,&source,Path::new("medium-spectrum.slang"),"mediumspectrum")?;
            assert_eq!(p.needs_spectral(&abi,&[])?,expected);
            let source=format!("struct Inputs {{}}; Closure surface(ShadingContext c,Inputs i) {{return unlit(float3({expression}));}}");
            let p=Compiler::discover().compile_surface(&abi,&source,Path::new("emitter-spectrum.slang"),"emitterspectrum")?;
            assert_eq!(p.needs_spectral(&abi,&[])?,expected);
        }
        Ok(())
    }
}

#[cfg(all(test, vork_private_tests))]
include!("camera_blend_spectral_tests_private.rs");

#[cfg(test)]
mod calibration_tests {
    use super::*;
    #[test]
    #[ignore = "requires pinned Slang"]
    fn std_camera_calibration_owns_optics() -> Result<()> {
        let abi=Abi {aov_count:0,context:Abi::spec_context(),closure:openpbr_fields()};
        let p=Compiler::discover().compile_camera(&abi,include_str!("../../../libraries/std/camera.slang"),Path::new("std/camera.slang"),"stdcamera")?;
        let words=p.inputs.pack(&BTreeMap::new(),|_,_|unreachable!())?;
        assert!(!p.uses_time(&abi)?);
        assert!(p.unit_weight(&abi,&words.iter().copied().map(Some).collect::<Vec<_>>())?);
        assert!(!p.needs_spectral(&abi,&words.iter().copied().map(Some).collect::<Vec<_>>())?);
        let c=p.calibration(&abi,&words,2.,-1.,0.)?.context("std calibration")?;
        assert_eq!(c[1],[0.1,1000.,0.,1.]);
        assert!((c[0][0]-100./36.).abs()<1e-6);
        assert!((c[0][1]-100./18.).abs()<1e-6);
        assert_eq!(c[2][0],0.);
        let c=p.calibration(&abi,&words,0.5,7.,1.)?.unwrap();
        assert_eq!(c[2][1],7.);
        assert!((c[0][1]-100./36.).abs()<1e-6);
        Ok(())
    }
}
