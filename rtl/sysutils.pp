{ Bruto Pascal runtime: a small part of FPC's SysUtils unit. Format is
  built into the compiler and uses BrutoCFormat from here. }
unit SysUtils;

interface

uses UnixType;

function IntToStr(Value: Int64): string;
function IntToHex(Value: Int64; Digits: Integer): string;
function StrToInt(const S: string): Int64;
function StrToIntDef(const S: string; Default: Int64): Int64;
function UpperCase(const S: string): string;
function LowerCase(const S: string): string;
function Trim(const S: string): string;
function TrimLeft(const S: string): string;
function TrimRight(const S: string): string;
function FileExists(const FileName: string): Boolean;
function DirectoryExists(const Directory: string): Boolean;
procedure Sleep(Milliseconds: LongWord);
{ A Pascal format string (`%d`, `%4.4x`, `%s`...) as the C one that
  prints the same with 64-bit integers. }
function BrutoCFormat(const Fmt: string): string;

implementation

function c_access(Path: PChar; Mode: cint): cint; cdecl; external 'c' name 'access';
function c_usleep(Micros: cuint): cint; cdecl; external 'c' name 'usleep';
function c_opendir(Path: PChar): Pointer; cdecl; external 'c' name 'opendir';
function c_closedir(Dir: Pointer): cint; cdecl; external 'c' name 'closedir';

function IntToStr(Value: Int64): string;
var
  S: string;
begin
  Str(Value, S);
  IntToStr := S;
end;

function IntToHex(Value: Int64; Digits: Integer): string;
begin
  IntToHex := HexStr(Value, Digits);
end;

function StrToIntDef(const S: string; Default: Int64): Int64;
var
  V: Int64;
  Code: Integer;
begin
  Val(Trim(S), V, Code);
  if Code = 0 then
    StrToIntDef := V
  else
    StrToIntDef := Default;
end;

function StrToInt(const S: string): Int64;
begin
  StrToInt := StrToIntDef(S, 0);
end;

function UpperCase(const S: string): string;
var
  I: Integer;
  R: string;
begin
  R := '';
  for I := 1 to Length(S) do
    if (S[I] >= 'a') and (S[I] <= 'z') then
      R := R + Chr(Ord(S[I]) - 32)
    else
      R := R + S[I];
  UpperCase := R;
end;

function LowerCase(const S: string): string;
var
  I: Integer;
  R: string;
begin
  R := '';
  for I := 1 to Length(S) do
    if (S[I] >= 'A') and (S[I] <= 'Z') then
      R := R + Chr(Ord(S[I]) + 32)
    else
      R := R + S[I];
  LowerCase := R;
end;

function TrimLeft(const S: string): string;
var
  I: Integer;
begin
  I := 1;
  while (I <= Length(S)) and (S[I] <= ' ') do
    I := I + 1;
  TrimLeft := Copy(S, I, Length(S) - I + 1);
end;

function TrimRight(const S: string): string;
var
  I: Integer;
begin
  I := Length(S);
  while (I > 0) and (S[I] <= ' ') do
    I := I - 1;
  TrimRight := Copy(S, 1, I);
end;

function Trim(const S: string): string;
begin
  Trim := TrimLeft(TrimRight(S));
end;

function FileExists(const FileName: string): Boolean;
begin
  FileExists := c_access(PChar(FileName), 0) = 0;
end;

function DirectoryExists(const Directory: string): Boolean;
var
  D: Pointer;
begin
  D := c_opendir(PChar(Directory));
  DirectoryExists := D <> nil;
  if D <> nil then
    c_closedir(D);
end;

procedure Sleep(Milliseconds: LongWord);
begin
  c_usleep(Milliseconds * 1000);
end;

function BrutoCFormat(const Fmt: string): string;
var
  I: Integer;
  R: string;
  C: Char;
begin
  R := '';
  I := 1;
  while I <= Length(Fmt) do
  begin
    C := Fmt[I];
    R := R + C;
    I := I + 1;
    if C = '%' then
    begin
      { Flags, width and precision carry over. }
      while (I <= Length(Fmt)) and (Fmt[I] in ['-', '0'..'9', '.', '*']) do
      begin
        R := R + Fmt[I];
        I := I + 1;
      end;
      if I <= Length(Fmt) then
      begin
        case Fmt[I] of
          'd', 'D': R := R + 'lld';
          'u', 'U': R := R + 'llu';
          'x', 'X': R := R + 'llX';
          'n', 'N', 'm', 'M': R := R + 'f';
        else
          R := R + LowerCase(Fmt[I]);
        end;
        I := I + 1;
      end;
    end;
  end;
  BrutoCFormat := R;
end;

end.
