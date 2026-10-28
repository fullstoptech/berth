-- P8-T5: folders Berth cloned from Git: the remote, branch, last deployed commit,
-- polling interval and Auto-redeploy switch (berth_core::ProjectSource::Git as json).
-- No credential is stored: ssh keys are named in the repository's own git config.
CREATE TABLE git_sources (folder TEXT PRIMARY KEY, engine TEXT NOT NULL, json TEXT NOT NULL);
