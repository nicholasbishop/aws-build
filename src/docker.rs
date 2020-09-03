use crate::command::run_cmd;
use anyhow::Error;
use fehler::throws;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Docker {
    cmd: String,
}

pub struct Volume {
    pub src: PathBuf,
    pub dst: PathBuf,
    pub read_only: bool,
}

impl Volume {
    fn arg(&self) -> OsString {
        let mut s = OsString::new();
        s.push(&self.src);
        s.push(":");
        s.push(&self.dst);
        if self.read_only {
            s.push(":ro");
        } else {
            s.push(":rw");
        }
        s
    }
}

impl Docker {
    pub fn new(cmd: String) -> Docker {
        Docker { cmd }
    }

    #[throws]
    pub fn build(&self, dir: &Path, image_tag: &str) {
        run_cmd(
            Command::new(&self.cmd)
                .current_dir(&dir)
                .args(&["build", "--tag", &image_tag, "."]),
        )?;
    }

    #[throws]
    pub fn run(&self, volumes: &[Volume], image_tag: &str) {
        let mut cmd = Command::new(&self.cmd);
        cmd.args(&["run", "--rm", "--init"]).arg("-u").arg(format!(
            "{}:{}",
            users::get_current_uid(),
            users::get_current_gid()
        ));

        for volume in volumes {
            cmd.arg("-v");
            cmd.arg(volume.arg());
        }
        cmd.arg(image_tag);

        run_cmd(&mut cmd)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_arg() {
        let mut vol = Volume {
            src: "/mySrc".into(),
            dst: "/myDst".into(),
            read_only: false,
        };
        assert_eq!(vol.arg(), "/mySrc:/myDst:rw");

        vol.read_only = true;
        assert_eq!(vol.arg(), "/mySrc:/myDst:ro");
    }
}
