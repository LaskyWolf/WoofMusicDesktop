#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayerConfig {
    pub sig: Option<String>,
    pub n_class: Option<String>,
    pub sts: Option<i64>,
    pub aliases: Option<Vec<String>>,
}

impl PlayerConfig {
    pub fn sig_function(&self) -> ExtractedFunction {
        ExtractedFunction {
            body: self.sig.clone(),
            ..ExtractedFunction::default()
        }
    }

    pub fn n_function(&self) -> ExtractedFunction {
        ExtractedFunction {
            var_name: self.n_class.clone(),
            ..ExtractedFunction::default()
        }
    }

    /// n-transform IIFE built from nClass.
    pub fn n_js_expression(&self) -> Option<String> {
        let n_class = self.n_class.as_deref()?;
        Some(format!(
            "(function(n){{try{{var u=new g.{n_class}('https://x.googlevideo.com/videoplayback?n='+n,true);\
             var t=u.get('n');return(t&&t!==n)?t:n;}}catch(e){{return n;}}}})(INPUT)"
        ))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedFunction {
    pub body: Option<String>,
    pub extract_pattern: Option<String>,
    pub var_name: Option<String>,
}
