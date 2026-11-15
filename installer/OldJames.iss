[Setup]
AppId={{DB656425-A6DD-43CB-9B71-61C5514A534E}
AppName=Old James
AppVersion=0.1.0
AppPublisher=Ian Rastall
DefaultDirName={localappdata}\Programs\Old James Cards
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.26100
OutputDir=..\dist
OutputBaseFilename=Old-James-Setup
SetupIconFile=..\assets\OldJames.ico
Compression=lzma2/fast
SolidCompression=yes
UninstallDisplayName=Old James (Cards)
UninstallDisplayIcon={app}\OldJames.Cards.exe
CloseApplications=yes

[Files]
Source: "..\.build\publish\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\Old James\Old James"; Filename: "{app}\OldJames.Cards.exe"; WorkingDir: "{app}"; IconFilename: "{app}\OldJames.ico"

[UninstallDelete]
Type: files; Name: "{app}\model-path.txt"

[Code]
procedure CurStepChanged(CurStep: TSetupStep);
var
  ModelPath: String;
  Lines: TArrayOfString;
begin
  if CurStep <> ssPostInstall then
    exit;
  ModelPath := ExpandConstant('{param:MODELFILE|}');
  if (ModelPath = '') or (not FileExists(ModelPath)) then
    exit;
  SetArrayLength(Lines, 1);
  Lines[0] := ModelPath;
  if not SaveStringsToUTF8File(ExpandConstant('{app}\model-path.txt'), Lines, False) then
    RaiseException('Could not save the initial model path.');
end;
