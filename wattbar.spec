Name:           wattbar
Version:        1.0.0
Release:        1%{?dist}
Summary:        Minimalist battery monitor for Wayland

License:        MIT
URL:            https://github.com/thequux/wattbar
Source0:        wattbar-1.0.0.tar.gz

BuildRequires:  cargo rust
#Requires:       

%description
Wattbar is the latest in a long line of minimalist battery monitors, the first
of which was xbattbar by YAMAGUCHI Suguru. It displays the battery charge level
and status in a thin line along one border of your screen. Wattbar
distinguishes itself by supporting Wayland (in particular, compositors that
support zwlr_layer_shell_v1).

%prep
%setup -q
#autosetup


%build
#configure
#make_build
cargo build --release


%install
mkdir -p %{buildroot}%{_bindir} %{buildroot}%{_datadir}/wattbar
install -m 0755 target/release/wattbar %{buildroot}%{_bindir}/wattbar
install -m 0644 default.theme %{buildroot}%{_datadir}/wattbar/default.theme

%files
%license COPYING
#doc add-docs-here
%{_bindir}/wattbar
%{_datadir}/wattbar/default.theme

%changelog
* Tue May 07 2024 TQ Hirsch <thequux@thequux.com>
- 
