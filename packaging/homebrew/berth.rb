# Homebrew cask template for Berth (P10-T1, PLAN §12 item 5).
# Not published from this repository: the owner copies it into a tap after the GitHub release
# exists. `scripts/release.sh` prints the version and the sha256 to paste below.
#   version  the release version without a leading "v", for example "1.0.0"
#   sha256   `shasum -a 256 Berth-<version>-arm64.dmg`
cask "berth" do
  version "0.0.0" # TODO: set at release
  sha256 "0000000000000000000000000000000000000000000000000000000000000000" # TODO: set at release

  url "https://github.com/fullstoptech/berth/releases/download/v#{version}/Berth-#{version}-arm64.dmg",
      verified: "github.com/fullstoptech/berth/"
  name "Berth"
  desc "Docker manager for Colima on macOS"
  homepage "https://github.com/fullstoptech/berth"

  livecheck do
    url :url
    strategy :github_latest
  end

  auto_updates false
  depends_on arch: :arm64
  depends_on macos: ">= :sonoma"

  app "Berth.app"

  zap trash: [
    "~/Library/Application Support/dev.berth.Berth",
    "~/Library/Caches/dev.berth.app",
    "~/Library/HTTPStorages/dev.berth.app",
    "~/Library/Logs/Berth",
    "~/Library/Preferences/dev.berth.app.plist",
    "~/Library/Saved Application State/dev.berth.app.savedState",
    "~/Library/WebKit/dev.berth.app",
  ]
end
