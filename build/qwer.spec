Name:           qwer
Version:        %(echo $CRATE_VERSION)
Release:        1%{?dist}
Summary:        A simple web app

License:        AGPLv3
Source0:        %{name}.tar.gz

BuildRequires:  systemd-rpm-macros

Provides:       %{name} = %{version}

%description
A simple web app

%global debug_package %{nil}

%prep
%autosetup


%build

%install
install -Dpm 0755 ./target/x86_64-unknown-linux-musl/release/%{name} %{buildroot}%{_bindir}/%{name}
install -Dpm 644 ./build/%{name}.service %{buildroot}%{_unitdir}/%{name}.service

mkdir -p %{buildroot}%{_datadir}/%{name}/www
mkdir -p %{buildroot}%{_datadir}/%{name}/www/static
mkdir -p %{buildroot}%{_datadir}/%{name}/www/help
cp -rf ./qw-site/frontend/dist/* %{buildroot}%{_datadir}/%{name}/www/static
cp -rf ./qw-site/docs/output/* %{buildroot}%{_datadir}/%{name}/www/help
find %{buildroot}%{_datadir}/%{name}/www -type f -exec chmod 644 {} \;
find %{buildroot}%{_datadir}/%{name}/www -type d -exec chmod 755 {} \;

%check
# go test should be here... :)

%post
%systemd_post %{name}.service

%preun
%systemd_preun %{name}.service

%files
%{_bindir}/%{name}
%{_unitdir}/%{name}.service
%{_datadir}/%{name}/www

%changelog
* Wed May 19 2021 John Doe - 1.0-1
- First release%changelog
