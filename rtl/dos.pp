{ Bruto Pascal runtime: the part of the Turbo Pascal / FPC Dos unit Free
  Vision uses, on the C library. }
unit Dos;

interface

uses UnixType;

const
  ReadOnly = $01;
  Hidden = $02;
  SysFile = $04;
  VolumeID = $08;
  Directory = $10;
  Archive = $20;
  AnyFile = $3F;
  FileNameLen = 255;

type
  ComStr = string[255];
  PathStr = string[255];
  DirStr = string[255];
  NameStr = string[255];
  ExtStr = string[255];

  DateTime = record
    Year, Month, Day, Hour, Min, Sec: Word;
  end;

  SearchRec = record
    { The search: an open directory, the directory's path (ending in
      '/'), the name pattern and the attributes asked for. }
    SearchHandle: Pointer;
    SearchDir: string[255];
    SearchMask: string[255];
    SearchAttr: Word;
    { The entry found. }
    Attr: Byte;
    Time: LongInt;
    Size: Int64;
    Name: string[255];
  end;

var
  DosError: Integer;

function GetEnv(EnvVar: string): string;
procedure Exec(const Path: PathStr; const ComLine: ComStr);
procedure SwapVectors;
function DosExitCode: Word;
procedure GetDate(var Year, Month, MDay, WDay: Word);
procedure GetTime(var Hour, Minute, Second, Sec100: Word);
procedure UnpackTime(P: LongInt; var T: DateTime);
procedure PackTime(var T: DateTime; var P: LongInt);
procedure FSplit(Path: PathStr; var Dir: DirStr; var Name: NameStr; var Ext: ExtStr);
function FExpand(const Path: PathStr): PathStr;
function FSearch(Path: PathStr; DirList: string): PathStr;
procedure FindFirst(const Path: PathStr; Attr: Word; var F: SearchRec);
procedure FindNext(var F: SearchRec);
procedure FindClose(var F: SearchRec);

implementation

type
  TTm = record
    tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year, tm_wday, tm_yday,
    tm_isdst: cint;
    tm_gmtoff: clong;
    tm_zone: PChar;
  end;
  TTimeVal = record
    tv_sec: time_t;
    tv_usec: suseconds_t;
  end;

function c_getenv(Name: PChar): PChar; cdecl; external 'c' name 'getenv';
function c_system(Command: PChar): cint; cdecl; external 'c' name 'system';
function c_gettimeofday(Tv: Pointer; Tz: Pointer): cint; cdecl; external 'c' name 'gettimeofday';
function c_localtime_r(T: Pointer; Tm: Pointer): Pointer; cdecl; external 'c' name 'localtime_r';
function c_getcwd(Buf: PChar; Size: size_t): PChar; cdecl; external 'c' name 'getcwd';
function c_access(Path: PChar; Mode: cint): cint; cdecl; external 'c' name 'access';
function c_opendir(Path: PChar): Pointer; cdecl; external 'c' name 'opendir';
function c_readdir(Dir: Pointer): Pointer; cdecl; external 'c' name 'readdir';
function c_closedir(Dir: Pointer): cint; cdecl; external 'c' name 'closedir';
function c_stat(Path: PChar; Buf: Pointer): cint; cdecl; external 'c' name 'stat';
function c_fnmatch(Pattern, Name: PChar; Flags: cint): cint; cdecl; external 'c' name 'fnmatch';

const
  { Offsets in struct dirent and struct stat. }
{$ifdef DARWIN}
  DirentName = 21;
  StatMode = 4;
  StatMTime = 48;
  StatSize = 96;
{$else}
  { Linux (glibc): st_mode follows st_nlink on x86-64, st_ino on arm64. }
  DirentName = 19;
{$ifdef CPUX86_64}
  StatMode = 24;
{$else}
  StatMode = 16;
{$endif}
  StatMTime = 88;
  StatSize = 48;
{$endif}
  S_IFMT = $F000;
  S_IFDIR = $4000;

var
  LastExitCode: Word;

function GetEnv(EnvVar: string): string;
var
  P: PChar;
begin
  P := c_getenv(PChar(EnvVar));
  if P = nil then
    GetEnv := ''
  else
    GetEnv := P;
end;

procedure Exec(const Path: PathStr; const ComLine: ComStr);
var
  Command: string;
  Status: cint;
begin
  Command := Path;
  if ComLine <> '' then
    Command := Command + ' ' + ComLine;
  Status := c_system(PChar(Command));
  if Status = -1 then
  begin
    DosError := 8;
    LastExitCode := 0;
  end
  else
  begin
    DosError := 0;
    LastExitCode := (Status shr 8) and $FF;
  end;
end;

procedure SwapVectors;
begin
end;

function DosExitCode: Word;
begin
  DosExitCode := LastExitCode;
end;

procedure Now(var Tm: TTm; var Usec: LongInt);
var
  Tv: TTimeVal;
begin
  c_gettimeofday(@Tv, nil);
  c_localtime_r(@Tv.tv_sec, @Tm);
  Usec := Tv.tv_usec;
end;

procedure GetDate(var Year, Month, MDay, WDay: Word);
var
  Tm: TTm;
  Usec: LongInt;
begin
  Now(Tm, Usec);
  Year := Tm.tm_year + 1900;
  Month := Tm.tm_mon + 1;
  MDay := Tm.tm_mday;
  WDay := Tm.tm_wday;
end;

procedure GetTime(var Hour, Minute, Second, Sec100: Word);
var
  Tm: TTm;
  Usec: LongInt;
begin
  Now(Tm, Usec);
  Hour := Tm.tm_hour;
  Minute := Tm.tm_min;
  Second := Tm.tm_sec;
  Sec100 := Usec div 10000;
end;

{ The DOS packed date and time: bits 25..31 year - 1980, 21..24 month,
  16..20 day, 11..15 hour, 5..10 minute, 0..4 seconds / 2. }
procedure UnpackTime(P: LongInt; var T: DateTime);
begin
  T.Year := ((P shr 25) and $7F) + 1980;
  T.Month := (P shr 21) and $0F;
  T.Day := (P shr 16) and $1F;
  T.Hour := (P shr 11) and $1F;
  T.Min := (P shr 5) and $3F;
  T.Sec := (P and $1F) * 2;
end;

procedure PackTime(var T: DateTime; var P: LongInt);
begin
  P := (LongInt(T.Year - 1980) shl 25) or (LongInt(T.Month) shl 21)
    or (LongInt(T.Day) shl 16) or (LongInt(T.Hour) shl 11)
    or (LongInt(T.Min) shl 5) or (T.Sec div 2);
end;

procedure FSplit(Path: PathStr; var Dir: DirStr; var Name: NameStr; var Ext: ExtStr);
var
  I, Slash, Dot: Integer;
begin
  Slash := 0;
  for I := 1 to Length(Path) do
    if Path[I] = '/' then
      Slash := I;
  Dot := 0;
  for I := Length(Path) downto Slash + 1 do
    if (Dot = 0) and (Path[I] = '.') then
      Dot := I;
  Dir := Copy(Path, 1, Slash);
  if Dot > 0 then
  begin
    Name := Copy(Path, Slash + 1, Dot - Slash - 1);
    Ext := Copy(Path, Dot, Length(Path) - Dot + 1);
  end
  else
  begin
    Name := Copy(Path, Slash + 1, Length(Path) - Slash);
    Ext := '';
  end;
end;

function FExpand(const Path: PathStr): PathStr;
var
  Buf: array[0..1023] of Char;
  Cwd: string;
begin
  if (Length(Path) > 0) and (Path[1] = '/') then
    FExpand := Path
  else if c_getcwd(@Buf, 1024) <> nil then
  begin
    Cwd := PChar(@Buf);
    if Path = '' then
      FExpand := Cwd
    else
      FExpand := Cwd + '/' + Path;
  end
  else
    FExpand := Path;
end;

function FSearch(Path: PathStr; DirList: string): PathStr;
var
  Start, I: Integer;
  Dir, Candidate: string;
begin
  FSearch := '';
  if c_access(PChar(string(Path)), 0) = 0 then
  begin
    FSearch := Path;
    Exit;
  end;
  Start := 1;
  for I := 1 to Length(DirList) + 1 do
    if (I > Length(DirList)) or (DirList[I] = ':') or (DirList[I] = ';') then
    begin
      Dir := Copy(DirList, Start, I - Start);
      Start := I + 1;
      if Dir <> '' then
      begin
        Candidate := Dir + '/' + Path;
        if c_access(PChar(Candidate), 0) = 0 then
        begin
          FSearch := Candidate;
          Exit;
        end;
      end;
    end;
end;

{ Fill F's entry fields from the file at Path; False if it isn't
  there. }
function StatEntry(const Path: string; var F: SearchRec): Boolean;
var
  Buf: array[0..255] of Byte;
  Mode: Word;
  MTime: Int64;
  Tm: TTm;
  DT: DateTime;
  DosTime: LongInt;
begin
  StatEntry := False;
  if c_stat(PChar(Path), @Buf) <> 0 then
    Exit;
  StatEntry := True;
  Mode := PWord(@Buf[StatMode])^;
  MTime := PInt64(@Buf[StatMTime])^;
  F.Size := PInt64(@Buf[StatSize])^;
  if (Mode and S_IFMT) = S_IFDIR then
    F.Attr := Directory
  else
    F.Attr := Archive;
  c_localtime_r(@MTime, @Tm);
  DT.Year := Tm.tm_year + 1900;
  DT.Month := Tm.tm_mon + 1;
  DT.Day := Tm.tm_mday;
  DT.Hour := Tm.tm_hour;
  DT.Min := Tm.tm_min;
  DT.Sec := Tm.tm_sec;
  PackTime(DT, DosTime);
  F.Time := DosTime;
end;

procedure FindFirst(const Path: PathStr; Attr: Word; var F: SearchRec);
var
  I, Slash: Integer;
  Dir: string;
begin
  F.SearchHandle := nil;
  F.SearchAttr := Attr;
  Slash := 0;
  for I := 1 to Length(Path) do
    if Path[I] = '/' then
      Slash := I;
  F.SearchDir := Copy(Path, 1, Slash);
  F.SearchMask := Copy(Path, Slash + 1, Length(Path) - Slash);
  { No wildcard: just the file itself. }
  if (Pos('*', F.SearchMask) = 0) and (Pos('?', F.SearchMask) = 0) then
  begin
    if StatEntry(Path, F) and (((F.Attr and Directory) = 0) or ((Attr and Directory) <> 0)) then
    begin
      F.Name := F.SearchMask;
      DosError := 0;
    end
    else
      DosError := 18;
    Exit;
  end;
  Dir := F.SearchDir;
  if Dir = '' then
    Dir := '.';
  F.SearchHandle := c_opendir(PChar(Dir));
  if F.SearchHandle = nil then
  begin
    DosError := 3;
    Exit;
  end;
  FindNext(F);
end;

procedure FindNext(var F: SearchRec);
var
  Entry: Pointer;
  Name: string;
begin
  DosError := 18;
  if F.SearchHandle = nil then
    Exit;
  repeat
    Entry := c_readdir(F.SearchHandle);
    if Entry = nil then
      Exit;
    Name := PChar(Entry) + DirentName;
    if (c_fnmatch(PChar(string(F.SearchMask)), PChar(Name), 0) = 0)
      and ((Name[1] <> '.') or ((F.SearchAttr and Hidden) <> 0) or (Name = '..'))
      and StatEntry(F.SearchDir + Name, F)
      and (((F.Attr and Directory) = 0) or ((F.SearchAttr and Directory) <> 0)) then
    begin
      F.Name := Name;
      DosError := 0;
      Exit;
    end;
  until False;
end;

procedure FindClose(var F: SearchRec);
begin
  if F.SearchHandle <> nil then
    c_closedir(F.SearchHandle);
  F.SearchHandle := nil;
end;

end.
