use shadow_rs::shadow;

shadow!(build_info);


pub const SHORT_VERSION: &str = shadow_rs::formatcp!("{} {} {}", build_info::LAST_TAG, build_info::SHORT_COMMIT, build_info::GIT_STATUS_FILE);
