/// JS string escaping. Internal: apps keep their own helper.
pub(crate) struct JSEscaping;

impl JSEscaping {
    pub(crate) fn escape(string: &str) -> String {
        string
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
    }
}
