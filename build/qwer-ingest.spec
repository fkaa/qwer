Name:           qwer-ingest
Version:        %(echo $CRATE_VERSION)
Release:        1%{?dist}
Summary:        A simple web app

License:        AGPLv3
Source0:        %{name}.tar.gz

BuildRequires:  systemd

Provides:       %{name} = %{version}

%description
A simple web app

%global debug_package %{nil}

%prep
%autosetup

%build

%install
install -Dpm 0755 ./build/%{name} %{buildroot}%{_bindir}/%{name}
install -Dpm 644 ./build/%{name}.service %{buildroot}%{_unitdir}/%{name}.service

%check
# go test should be here... :)

%post
%systemd_post %{name}.service

%preun
%systemd_preun %{name}.service

%files
%{_bindir}/%{name}
%{_unitdir}/%{name}.service

%changelog
* Wed May 19 2021 John Doe - 1.0-1
- First release%changelog
