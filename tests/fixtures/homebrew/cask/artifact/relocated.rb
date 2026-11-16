# Minimal platform boundary for the pinned Homebrew artifacts in these tests.
# Artifact link/conflict/unlink policy comes from the unmodified upstream files.
require "fileutils"
require "pathname"

module T
  module Helpers; end
end
module OS
  def self.mac?; false; end
end
module Cask
  class CaskError < StandardError; end
  module Utils
    def self.gain_permissions_remove(path, command:)
      path.unlink
    end
    def self.gain_permissions_mkpath(path, command:)
      FileUtils.mkdir_p(path)
    end
  end
  module Artifact
    class Relocated
      attr_reader :cask, :source, :target
      def self.sig(&block); end
      def self.english_name; name.split("::").last; end
      def self.english_article; "a"; end
      def initialize(cask, source, target: nil)
        @cask = cask
        @source = cask.staged_path/source
        @target = HOMEBREW_PREFIX/"bin"/(target || source)
      end
      def printable_target; target; end
      def ohai(*args); end
      def opoo(*args); end
      def odebug(*args); end
    end
  end
end
