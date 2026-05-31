{
  description = "HP OmniBook Ultra Flip settings application";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      packages.${system}.omnibook-rs = pkgs.rustPlatform.buildRustPackage {
        pname = "omnibook-rs";
        version = "0.1.0";

        src = ./.;
        cargoLock.lockFile = ./Cargo.lock;

        nativeBuildInputs = with pkgs; [ pkg-config wrapGAppsHook4 ];
        buildInputs = with pkgs; [ gtk4 libadwaita ];

        postInstall = ''
          install -Dm644 assets/hp-logo.svg \
            $out/share/icons/hicolor/scalable/apps/omnibook-rs.svg

          install -Dm644 assets/omnibook-rs.desktop \
            $out/share/applications/omnibook-rs.desktop

          install -Dm644 daemon/omnibookd.service \
            $out/lib/systemd/user/omnibookd.service
          substituteInPlace $out/lib/systemd/user/omnibookd.service \
            --replace-fail '@omnibookd@' "$out/bin/omnibookd"
        '';

        meta = {
          description = "HP OmniBook Ultra Flip settings application";
          license = pkgs.lib.licenses.mit;
          mainProgram = "omnibook-rs";
        };
      };

      nixosModules.default =
        { config, lib, pkgs, ... }:
        let
          cfg = config.programs.omnibook;
          omnibook = self.packages.${pkgs.system}.omnibook-rs;
        in
        {
          options.programs.omnibook.enable = lib.mkEnableOption "HP OmniBook settings application";

          config = lib.mkIf cfg.enable {
            environment.systemPackages = [ omnibook pkgs.brightnessctl ];

            services.udev.extraRules = ''
              SUBSYSTEM=="hidraw", KERNEL=="hidraw*", KERNELS=="0018:06CB:CFD2.*", MODE="0666", TAG+="uaccess"
            '';

            systemd.user.services.omnibookd = {
              description = "HP OmniBook daemon - sensor monitoring";
              documentation = [ "https://github.com/TimP4w/omnibook-rs" ];
              after = [ "graphical-session.target" ];
              wantedBy = [ "graphical-session.target" ];
              serviceConfig = {
                Type = "simple";
                ExecStart = "${omnibook}/bin/omnibookd";
                Restart = "on-failure";
                RestartSec = 5;
                Environment = "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/%U/bus";
                PassEnvironment = "XDG_CURRENT_DESKTOP XDG_SESSION_TYPE WAYLAND_DISPLAY DISPLAY SWAYSOCK HYPRLAND_INSTANCE_SIGNATURE";
              };
            };
          };
        };

      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          rustc
          cargo
          rust-analyzer
          rustfmt
          pkg-config
          gtk4
          libadwaita
          gobject-introspection
          brightnessctl
        ];

        RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";

        shellHook = ''
          export XDG_DATA_DIRS="${pkgs.gtk4}/share/gsettings-schemas/${pkgs.gtk4.name}:${pkgs.libadwaita}/share/gsettings-schemas/${pkgs.libadwaita.name}:$XDG_DATA_DIRS"
          echo "HP OmniBook Settings — run with: cargo run"
        '';
      };
    };
}
