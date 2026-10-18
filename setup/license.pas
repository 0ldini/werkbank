
[Code]
{ Lizenzcode: Installation nur mit gültigem Code. Verglichen werden Prüfsummen, nicht der Code.
  Die Platzhalter @...@ setzt setup/make_iss.py aus launcher/license.sha256 ein. }
const
  LicHashAnsi = '@CP1252@';
  LicHashUnicode = '@UTF16@';
  LicHashKey = '@UTF8@';

var
  LicPage: TInputQueryWizardPage;

function LicOk(S: String): Boolean;
begin
  S := Trim(S);
  Result := (S <> '') and ((GetSHA256OfUnicodeString(S) = LicHashUnicode) or (GetSHA256OfString(S) = LicHashAnsi));
end;

function InitializeSetup: Boolean;
begin
  Result := True;
  { Stille Installation nur mit /LICENSE=<code> }
  if WizardSilent then
    Result := LicOk(ExpandConstant('{param:LICENSE|}'));
end;

procedure InitializeWizard;
begin
  LicPage := CreateInputQueryPage(wpWelcome, 'Lizenzcode',
    'Bitte den Lizenzcode eingeben',
    'Die Werkbank lässt sich nur mit gültigem Lizenzcode installieren.');
  LicPage.Add('Lizenzcode:', False);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = LicPage.ID then
    if not LicOk(LicPage.Values[0]) then
    begin
      MsgBox('Der Lizenzcode ist ungültig.', mbError, MB_OK);
      Result := False;
    end;
end;

{ Merkt die Freischaltung für den Launcher (gleiche Datei wie im Launcher: license.key). }
procedure CurStepChanged(CurStep: TSetupStep);
var
  Dir: String;
begin
  if CurStep = ssPostInstall then
  begin
    Dir := ExpandConstant('{localappdata}\Werkbank\config');
    ForceDirectories(Dir);
    SaveStringToFile(Dir + '\license.key', LicHashKey, False);
  end;
end;
