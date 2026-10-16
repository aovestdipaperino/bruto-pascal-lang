{ Bruto Pascal runtime: TObject, the root of every class, and TClass,
  as FPC's objpas declares them. Added to every program that declares a
  class. In a program that has a TObject of its own (Free Vision's
  Objects unit declares an object type of that name), this one is
  BrutoTObject, and classes declared without a parent still descend
  from it.

  A class's VMT starts with the instance size, its negation, a link to
  the parent's VMT, the class name, the routine that releases the
  strings an instance holds and the table of the interfaces the class
  implements; the virtual methods follow. }
unit ObjPas;

interface

type
  TClass = class of TObject;

  PGUID = ^TGUID;
  TGUID = record
    D1: LongWord;
    D2: Word;
    D3: Word;
    D4: array[0..7] of Byte;
  end;

  PExtended = ^Double;
  PInt64 = ^Int64;
  PQWord = ^QWord;

  { An element of an `array of const` parameter: VType says which field
    holds the value (vtInteger: VInteger, vtAnsiString: VAnsiString...).
    Integers that don't fit a LongInt come as vtInt64. }
  PVarRec = ^TVarRec;
  TVarRec = record
    case VType: SizeInt of
      0: (VInteger: LongInt);
      1: (VBoolean: Boolean);
      2: (VChar: Char);
      3: (VExtended: PExtended);
      4: (VString: PShortString);
      5: (VPointer: Pointer);
      6: (VPChar: PChar);
      7: (VObject: TObject);
      8: (VClass: TClass);
      9: (VWideChar: WideChar);
      10: (VPWideChar: PWideChar);
      11: (VAnsiString: Pointer);
      16: (VInt64: PInt64);
      17: (VQWord: PQWord);
      18: (VUnicodeString: Pointer);
  end;

  { A method pointer's parts: `TMethod(OnClick).Data` is the instance. }
  TMethod = record
    Code, Data: Pointer;
  end;

  TObject = class
  public
    constructor Create;
    destructor Destroy; virtual;
    { Destroy, unless Self is nil. }
    procedure Free;
    class function ClassName: ShortString;
    class function ClassNameIs(const Name: AnsiString): Boolean;
    class function ClassParent: TClass;
    class function InstanceSize: SizeInt;
    class function InheritsFrom(AClass: TClass): Boolean;
    function ClassType: TClass;
    procedure AfterConstruction; virtual;
    procedure BeforeDestruction; virtual;
    function Equals(Obj: TObject): Boolean; virtual;
    function GetHashCode: PtrInt; virtual;
    function ToString: AnsiString; virtual;
    { Whether the class implements the interface whose GUID is IID; if
      so, Obj gets a reference to it. }
    function GetInterface(const IID: TGUID; out Obj): Boolean;
  end;

  IInterface = interface
    ['{00000000-0000-0000-C000-000000000046}']
    function QueryInterface(constref IID: TGUID; out Obj): LongInt;
    function _AddRef: LongInt;
    function _Release: LongInt;
  end;
  IUnknown = IInterface;

  { An object freed when the last interface reference to it goes. }
  TInterfacedObject = class(TObject, IUnknown)
  protected
    FRefCount: LongInt;
    function QueryInterface(constref IID: TGUID; out Obj): LongInt;
    function _AddRef: LongInt;
    function _Release: LongInt;
  public
    property RefCount: LongInt read FRefCount;
  end;

const
  S_OK = 0;
  E_NOINTERFACE = LongInt($80004002);

const
  vtInteger = 0;
  vtBoolean = 1;
  vtChar = 2;
  vtExtended = 3;
  vtString = 4;
  vtPointer = 5;
  vtPChar = 6;
  vtObject = 7;
  vtClass = 8;
  vtWideChar = 9;
  vtPWideChar = 10;
  vtAnsiString = 11;
  vtCurrency = 12;
  vtVariant = 13;
  vtInterface = 14;
  vtWideString = 15;
  vtInt64 = 16;
  vtQWord = 17;
  vtUnicodeString = 18;

var
  { The innermost exception frame (one per `try` being run). }
  BrutoExcTop: Pointer;
  { The exception being raised or handled. }
  BrutoExcObject: TObject;
  { The newest record of a running routine's string locals, which an
    exception leaving it releases. }
  BrutoCleanupTop: Pointer;

{ Raise Obj: go back to the innermost `try`, or report it and stop. }
procedure BrutoRaise(Obj: TObject; Address: Pointer);
{ Free a handled exception. }
procedure BrutoFreeException(Obj: TObject);

implementation

type
  PBrutoExcFrame = ^TBrutoExcFrame;
  TBrutoExcFrame = record
    Buf: array[0..63] of Int64;
    Prev: PBrutoExcFrame;
    Cleanup: Pointer;
  end;
  { A routine's string locals: Count slot addresses follow. }
  PBrutoCleanup = ^TBrutoCleanup;
  TBrutoCleanup = record
    Prev: PBrutoCleanup;
    Count: PtrInt;
  end;

{$ifdef WINDOWS}
procedure bruto_longjmp(Buf: Pointer; Value: LongInt); cdecl; external 'c' name 'longjmp';
{$else}
procedure bruto_longjmp(Buf: Pointer; Value: LongInt); cdecl; external 'c' name '_longjmp';
{$endif}
{$ifdef WINDOWS}
function bruto_exc_write(Fd: LongInt; Buf: PChar; Count: LongWord): LongInt; cdecl;
  external 'c' name '_write';
{$else}
function bruto_exc_write(Fd: LongInt; Buf: PChar; Count: PtrUInt): PtrInt; cdecl;
  external 'c' name 'write';
{$endif}
function bruto_exc_fflush(Stream: Pointer): LongInt; cdecl; external 'c' name 'fflush';

procedure BrutoUnhandled(Obj: TObject; Address: Pointer);
var
  Where, Report: AnsiString;
begin
  Where := '$' + HexStr(PtrUInt(Address), 16);
  Report := 'An unhandled exception occurred at ' + Where + ':'#10 + Obj.ToString + #10 +
    '  ' + Where + #10#10;
  bruto_exc_fflush(nil);
  bruto_exc_write(2, PChar(Report), Length(Report));
  Halt(217);
end;

procedure BrutoRaise(Obj: TObject; Address: Pointer);
var
  Frame: PBrutoExcFrame;
  Rec: PBrutoCleanup;
  Slot: Pointer;
  I: PtrInt;
begin
  Frame := BrutoExcTop;
  BrutoExcObject := Obj;
  if Frame = nil then
    BrutoUnhandled(Obj, Address);
  BrutoExcTop := Frame^.Prev;
  { Release the strings of the routines being left. }
  while BrutoCleanupTop <> Frame^.Cleanup do
  begin
    Rec := BrutoCleanupTop;
    for I := 0 to Rec^.Count - 1 do
    begin
      Slot := PPointer(PtrUInt(Rec) + SizeOf(TBrutoCleanup) + PtrUInt(I) * SizeOf(Pointer))^;
      bruto_astr_release(PPointer(Slot)^);
      PPointer(Slot)^ := nil;
    end;
    BrutoCleanupTop := Rec^.Prev;
  end;
  bruto_longjmp(@Frame^.Buf, 1);
end;

procedure BrutoFreeException(Obj: TObject);
begin
  if BrutoExcObject = Obj then
    BrutoExcObject := nil;
  Obj.Free;
end;

type
  PBrutoClassHeader = ^TBrutoClassHeader;
  TBrutoClassHeader = record
    InstanceSize: SizeInt;
    NegInstanceSize: SizeInt;
    ParentRef: PPointer;
    ClassName: PShortString;
    Cleanup: Pointer;
  end;

constructor TObject.Create;
begin
end;

destructor TObject.Destroy;
begin
end;

procedure TObject.Free;
begin
  if Self <> nil then
    Destroy;
end;

class function TObject.ClassName: ShortString;
begin
  ClassName := PBrutoClassHeader(Self)^.ClassName^;
end;

class function TObject.ClassNameIs(const Name: AnsiString): Boolean;
var
  Own: ShortString;
  I: Integer;
begin
  Own := ClassName;
  ClassNameIs := Length(Own) = Length(Name);
  I := 1;
  while ClassNameIs and (I <= Length(Name)) do
  begin
    ClassNameIs := UpCase(Own[I]) = UpCase(Name[I]);
    I := I + 1;
  end;
end;

class function TObject.ClassParent: TClass;
var
  Ref: PPointer;
begin
  Ref := PBrutoClassHeader(Self)^.ParentRef;
  if Ref = nil then
    ClassParent := nil
  else
    ClassParent := TClass(Ref^);
end;

class function TObject.InstanceSize: SizeInt;
begin
  InstanceSize := PBrutoClassHeader(Self)^.InstanceSize;
end;

class function TObject.InheritsFrom(AClass: TClass): Boolean;
var
  C: TClass;
begin
  C := Self;
  while (C <> nil) and (C <> AClass) do
    C := C.ClassParent;
  InheritsFrom := C <> nil;
end;

function TObject.ClassType: TClass;
begin
  ClassType := TClass(PPointer(Self)^);
end;

procedure TObject.AfterConstruction;
begin
end;

procedure TObject.BeforeDestruction;
begin
end;

function TObject.Equals(Obj: TObject): Boolean;
begin
  Equals := Obj = Self;
end;

function TObject.GetHashCode: PtrInt;
begin
  GetHashCode := PtrInt(Self);
end;

function TObject.ToString: AnsiString;
begin
  ToString := ClassName;
end;

type
  PBrutoWords = ^TBrutoWords;
  TBrutoWords = array[0..$FFFFFF] of Pointer;
  PBrutoGUIDBytes = ^TBrutoGUIDBytes;
  TBrutoGUIDBytes = array[0..15] of Byte;
  TBrutoIntfCount = function(Obj: Pointer): LongInt;

function TObject.GetInterface(const IID: TGUID; out Obj): Boolean;
var
  Table, Desc, ITable: Pointer;
  Count, I, J: Int64;
  Same: Boolean;
  AddRef: TBrutoIntfCount;
begin
  GetInterface := False;
  Table := PBrutoWords(PBrutoWords(Pointer(Self))^[0])^[5];
  if Table = nil then
    Exit;
  Count := PInt64(Table)^;
  I := 0;
  while I < Count do
  begin
    Desc := PBrutoWords(Table)^[1 + I * 2];
    Same := True;
    for J := 0 to 15 do
      if PBrutoGUIDBytes(Desc)^[J] <> PBrutoGUIDBytes(@IID)^[J] then
        Same := False;
    if Same then
    begin
      ITable := PBrutoWords(Table)^[2 + I * 2];
      PBrutoWords(@Obj)^[0] := Pointer(Self);
      PBrutoWords(@Obj)^[1] := ITable;
      AddRef := TBrutoIntfCount(PBrutoWords(ITable)^[1]);
      AddRef(Pointer(Self));
      GetInterface := True;
      Exit;
    end;
    I := I + 1;
  end;
end;

function TInterfacedObject.QueryInterface(constref IID: TGUID; out Obj): LongInt;
begin
  if GetInterface(IID, Obj) then
    QueryInterface := S_OK
  else
    QueryInterface := E_NOINTERFACE;
end;

function TInterfacedObject._AddRef: LongInt;
begin
  FRefCount := FRefCount + 1;
  _AddRef := FRefCount;
end;

function TInterfacedObject._Release: LongInt;
begin
  FRefCount := FRefCount - 1;
  _Release := FRefCount;
  if FRefCount = 0 then
    Destroy;
end;

end.
