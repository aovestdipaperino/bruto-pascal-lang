{ Bruto Pascal runtime: the core of FPC's Classes unit. Lists (TFPList,
  TList), string lists (TStrings, TStringList) and streams (TStream,
  THandleStream, TFileStream, TMemoryStream, TStringStream), with FPC's
  interfaces and messages. }
unit Classes;

{$mode objfpc}{$H+}

interface

uses SysUtils;

type
  TNotifyEvent = procedure(Sender: TObject) of object;

  EListError = class(Exception);
  EStringListError = class(Exception);
  EStreamError = class(Exception);
  EFCreateError = class(EStreamError);
  EFOpenError = class(EStreamError);
  EReadError = class(EStreamError);
  EWriteError = class(EStreamError);

  { Pointer lists. }
  PPointerList = ^TPointerList;
  TPointerList = array[0..MaxInt div 16] of Pointer;
  TListSortCompare = function(Item1, Item2: Pointer): Integer;

  TFPList = class(TObject)
  private
    FList: PPointerList;
    FCount: Integer;
    FCapacity: Integer;
    function Get(Index: Integer): Pointer;
    procedure Put(Index: Integer; Item: Pointer);
    procedure SetCapacity(NewCapacity: Integer);
    procedure SetCount(NewCount: Integer);
    procedure CheckIndex(Index: Integer);
  public
    destructor Destroy; override;
    function Add(Item: Pointer): Integer;
    procedure Clear;
    procedure Delete(Index: Integer);
    procedure Exchange(Index1, Index2: Integer);
    function Expand: TFPList;
    function Extract(Item: Pointer): Pointer;
    function First: Pointer;
    function IndexOf(Item: Pointer): Integer;
    procedure Insert(Index: Integer; Item: Pointer);
    function Last: Pointer;
    procedure Move(CurIndex, NewIndex: Integer);
    procedure Pack;
    function Remove(Item: Pointer): Integer;
    procedure Sort(Compare: TListSortCompare);
    procedure Assign(ListA: TFPList);
    property Capacity: Integer read FCapacity write SetCapacity;
    property Count: Integer read FCount write SetCount;
    property Items[Index: Integer]: Pointer read Get write Put; default;
    property List: PPointerList read FList;
  end;

  TList = class(TObject)
  private
    FList: TFPList;
    function Get(Index: Integer): Pointer;
    procedure Put(Index: Integer; Item: Pointer);
    function GetCount: Integer;
    procedure SetCount(NewCount: Integer);
    function GetCapacity: Integer;
    procedure SetCapacity(NewCapacity: Integer);
  public
    constructor Create;
    destructor Destroy; override;
    function Add(Item: Pointer): Integer;
    procedure Clear; virtual;
    procedure Delete(Index: Integer);
    procedure Exchange(Index1, Index2: Integer);
    function Expand: TList;
    function Extract(Item: Pointer): Pointer;
    function First: Pointer;
    function IndexOf(Item: Pointer): Integer;
    procedure Insert(Index: Integer; Item: Pointer);
    function Last: Pointer;
    procedure Move(CurIndex, NewIndex: Integer);
    procedure Pack;
    function Remove(Item: Pointer): Integer;
    procedure Sort(Compare: TListSortCompare);
    property Capacity: Integer read GetCapacity write SetCapacity;
    property Count: Integer read GetCount write SetCount;
    property Items[Index: Integer]: Pointer read Get write Put; default;
  end;

  { Compiled with type information: its descendants' default sections
    are published, as in FPC. }
{$M+}
  TPersistent = class(TObject)
  protected
    procedure AssignTo(Dest: TPersistent); virtual;
  public
    procedure Assign(Source: TPersistent); virtual;
  end;
{$M-}

  { A component: owned components are freed with their owner. }
  TComponent = class(TPersistent)
  private
    FOwner: TComponent;
    FName: string;
    FTag: PtrInt;
    FComponents: TFPList;
    function GetComponent(Index: Integer): TComponent;
    function GetComponentCount: Integer;
  public
    constructor Create(AOwner: TComponent); virtual;
    destructor Destroy; override;
    procedure InsertComponent(AComponent: TComponent);
    procedure RemoveComponent(AComponent: TComponent);
    function FindComponent(const AName: string): TComponent;
    property Owner: TComponent read FOwner;
    property Components[Index: Integer]: TComponent read GetComponent;
    property ComponentCount: Integer read GetComponentCount;
  published
    property Name: string read FName write FName stored False;
    property Tag: PtrInt read FTag write FTag default 0;
  end;
  TComponentClass = class of TComponent;

  TSeekOrigin = (soBeginning, soCurrent, soEnd);

  TStream = class(TObject)
  protected
    function GetPosition: Int64; virtual;
    procedure SetPosition(const Pos: Int64); virtual;
    function GetSize: Int64; virtual;
    procedure SetSize(const NewSize: Int64); virtual;
  public
    function Read(var Buffer; Count: LongInt): LongInt; virtual; abstract;
    function Write(const Buffer; Count: LongInt): LongInt; virtual; abstract;
    function Seek(const Offset: Int64; Origin: TSeekOrigin): Int64; virtual; abstract;
    procedure ReadBuffer(var Buffer; Count: LongInt);
    procedure WriteBuffer(const Buffer; Count: LongInt);
    function CopyFrom(Source: TStream; Count: Int64): Int64;
    function ReadByte: Byte;
    procedure WriteByte(B: Byte);
    function ReadAnsiString: AnsiString;
    procedure WriteAnsiString(const S: AnsiString);
    property Position: Int64 read GetPosition write SetPosition;
    property Size: Int64 read GetSize write SetSize;
  end;

  THandleStream = class(TStream)
  private
    FHandle: THandle;
  protected
    procedure SetSize(const NewSize: Int64); override;
  public
    constructor Create(AHandle: THandle);
    function Read(var Buffer; Count: LongInt): LongInt; override;
    function Write(const Buffer; Count: LongInt): LongInt; override;
    function Seek(const Offset: Int64; Origin: TSeekOrigin): Int64; override;
    property Handle: THandle read FHandle;
  end;

  TFileStream = class(THandleStream)
  private
    FFileName: string;
  public
    constructor Create(const AFileName: string; Mode: Word);
    destructor Destroy; override;
    property FileName: string read FFileName;
  end;

  TCustomMemoryStream = class(TStream)
  private
    FMemory: Pointer;
    FSize: PtrInt;
    FPosition: PtrInt;
  protected
    procedure SetPointer(Ptr: Pointer; ASize: PtrInt);
  public
    function Read(var Buffer; Count: LongInt): LongInt; override;
    function Seek(const Offset: Int64; Origin: TSeekOrigin): Int64; override;
    procedure SaveToStream(Stream: TStream);
    procedure SaveToFile(const FileName: string);
    property Memory: Pointer read FMemory;
  end;

  TMemoryStream = class(TCustomMemoryStream)
  private
    FCapacity: PtrInt;
    procedure SetCapacity(NewCapacity: PtrInt);
  protected
    procedure SetSize(const NewSize: Int64); override;
  public
    destructor Destroy; override;
    procedure Clear;
    procedure LoadFromStream(Stream: TStream);
    procedure LoadFromFile(const FileName: string);
    function Write(const Buffer; Count: LongInt): LongInt; override;
  end;

  TStringStream = class(TMemoryStream)
  private
    function GetDataString: string;
  public
    constructor Create(const AString: string);
    function ReadString(Count: LongInt): string;
    procedure WriteString(const AString: string);
    property DataString: string read GetDataString;
  end;

  { String lists. }
  TStringsEnumerator = class;

  TStrings = class(TPersistent)
  private
    FUpdateCount: Integer;
    FDelimiter: Char;
    FQuoteChar: Char;
    FNameValueSeparator: Char;
    FStrictDelimiter: Boolean;
    FLineBreak: string;
    function GetCommaText: string;
    procedure SetCommaText(const Value: string);
    function GetDelimitedText: string;
    procedure SetDelimitedText(const Value: string);
    function GetName(Index: Integer): string;
    function GetValue(const Name: string): string;
    procedure SetValue(const Name, Value: string);
    function GetValueFromIndex(Index: Integer): string;
    procedure SetValueFromIndex(Index: Integer; const Value: string);
  protected
    function Get(Index: Integer): string; virtual; abstract;
    function GetCount: Integer; virtual; abstract;
    function GetObject(Index: Integer): TObject; virtual;
    procedure Put(Index: Integer; const S: string); virtual;
    procedure PutObject(Index: Integer; AObject: TObject); virtual;
    function GetTextStr: string; virtual;
    procedure SetTextStr(const Value: string); virtual;
    procedure SetUpdateState(Updating: Boolean); virtual;
    function DoCompareText(const S1, S2: string): Integer; virtual;
    property UpdateCount: Integer read FUpdateCount;
  public
    constructor Create;
    function Add(const S: string): Integer; virtual;
    function AddObject(const S: string; AObject: TObject): Integer; virtual;
    procedure AddStrings(TheStrings: TStrings); virtual;
    procedure Append(const S: string);
    procedure Assign(Source: TPersistent); override;
    procedure BeginUpdate;
    procedure EndUpdate;
    procedure Clear; virtual; abstract;
    procedure Delete(Index: Integer); virtual; abstract;
    procedure Insert(Index: Integer; const S: string); virtual; abstract;
    procedure InsertObject(Index: Integer; const S: string; AObject: TObject);
    procedure Exchange(Index1, Index2: Integer); virtual;
    procedure Move(CurIndex, NewIndex: Integer); virtual;
    function Equals(Obj: TObject): Boolean; override;
    function IndexOf(const S: string): Integer; virtual;
    function IndexOfName(const Name: string): Integer; virtual;
    function IndexOfObject(AObject: TObject): Integer; virtual;
    procedure LoadFromFile(const FileName: string); virtual;
    procedure LoadFromStream(Stream: TStream); virtual;
    procedure SaveToFile(const FileName: string); virtual;
    procedure SaveToStream(Stream: TStream); virtual;
    function GetEnumerator: TStringsEnumerator;
    property Count: Integer read GetCount;
    property Strings[Index: Integer]: string read Get write Put; default;
    property Objects[Index: Integer]: TObject read GetObject write PutObject;
    property Names[Index: Integer]: string read GetName;
    property Values[const Name: string]: string read GetValue write SetValue;
    property ValueFromIndex[Index: Integer]: string read GetValueFromIndex write SetValueFromIndex;
    property Text: string read GetTextStr write SetTextStr;
    property CommaText: string read GetCommaText write SetCommaText;
    property DelimitedText: string read GetDelimitedText write SetDelimitedText;
    property Delimiter: Char read FDelimiter write FDelimiter;
    property QuoteChar: Char read FQuoteChar write FQuoteChar;
    property NameValueSeparator: Char read FNameValueSeparator write FNameValueSeparator;
    property StrictDelimiter: Boolean read FStrictDelimiter write FStrictDelimiter;
    property LineBreak: string read FLineBreak write FLineBreak;
  end;

  TStringsEnumerator = class
  private
    FStrings: TStrings;
    FPosition: Integer;
  public
    constructor Create(AStrings: TStrings);
    function GetCurrent: string;
    function MoveNext: Boolean;
    property Current: string read GetCurrent;
  end;

  TDuplicates = (dupIgnore, dupAccept, dupError);

  TStringList = class;
  TStringListSortCompare = function(List: TStringList; Index1, Index2: Integer): Integer;

  TStringItem = record
    FString: string;
    FObject: TObject;
  end;
  PStringItemList = ^TStringItemList;
  TStringItemList = array[0..MaxInt div 64] of TStringItem;

  TStringList = class(TStrings)
  private
    FList: PStringItemList;
    FCount: Integer;
    FCapacity: Integer;
    FSorted: Boolean;
    FDuplicates: TDuplicates;
    FCaseSensitive: Boolean;
    FOwnsObjects: Boolean;
    FOnChange: TNotifyEvent;
    FOnChanging: TNotifyEvent;
    procedure Grow;
    procedure SetSorted(Value: Boolean);
    procedure SetCaseSensitive(Value: Boolean);
    procedure QuickSort(L, R: Integer; Compare: TStringListSortCompare);
    procedure CheckIndex(Index: Integer);
  protected
    procedure Changed; virtual;
    procedure Changing; virtual;
    function Get(Index: Integer): string; override;
    function GetCount: Integer; override;
    function GetObject(Index: Integer): TObject; override;
    procedure Put(Index: Integer; const S: string); override;
    procedure PutObject(Index: Integer; AObject: TObject); override;
    procedure SetCapacity(NewCapacity: Integer);
    procedure SetUpdateState(Updating: Boolean); override;
    procedure InsertItem(Index: Integer; const S: string; O: TObject);
    function DoCompareText(const S1, S2: string): Integer; override;
  public
    destructor Destroy; override;
    function Add(const S: string): Integer; override;
    function AddObject(const S: string; AObject: TObject): Integer; override;
    procedure Clear; override;
    procedure Delete(Index: Integer); override;
    procedure Exchange(Index1, Index2: Integer); override;
    function Find(const S: string; var Index: Integer): Boolean; virtual;
    function IndexOf(const S: string): Integer; override;
    procedure Insert(Index: Integer; const S: string); override;
    procedure Sort; virtual;
    procedure CustomSort(CompareFn: TStringListSortCompare); virtual;
    property Capacity: Integer read FCapacity write SetCapacity;
    property Duplicates: TDuplicates read FDuplicates write FDuplicates;
    property Sorted: Boolean read FSorted write SetSorted;
    property CaseSensitive: Boolean read FCaseSensitive write SetCaseSensitive;
    property OwnsObjects: Boolean read FOwnsObjects write FOwnsObjects;
    property OnChange: TNotifyEvent read FOnChange write FOnChange;
    property OnChanging: TNotifyEvent read FOnChanging write FOnChanging;
  end;

const
  fmCreate = $FF00;

implementation

function ListIndexError(Index: Integer): EListError;
begin
  ListIndexError := EListError.Create('List index (' + IntToStr(Index) + ') out of bounds');
end;

{ TFPList }

destructor TFPList.Destroy;
begin
  Clear;
  inherited Destroy;
end;

procedure TFPList.CheckIndex(Index: Integer);
begin
  if (Index < 0) or (Index >= FCount) then
    raise ListIndexError(Index);
end;

function TFPList.Get(Index: Integer): Pointer;
begin
  CheckIndex(Index);
  Result := FList^[Index];
end;

procedure TFPList.Put(Index: Integer; Item: Pointer);
begin
  CheckIndex(Index);
  FList^[Index] := Item;
end;

procedure TFPList.SetCapacity(NewCapacity: Integer);
begin
  if NewCapacity < FCount then
    raise EListError.Create('List capacity (' + IntToStr(NewCapacity) + ') exceeded.');
  if NewCapacity = FCapacity then
    Exit;
  ReallocMem(FList, NewCapacity * SizeOf(Pointer));
  FCapacity := NewCapacity;
end;

procedure TFPList.SetCount(NewCount: Integer);
begin
  if NewCount < 0 then
    raise EListError.Create('List count (' + IntToStr(NewCount) + ') out of bounds.');
  if NewCount > FCapacity then
    SetCapacity(NewCount);
  if NewCount > FCount then
    FillChar(FList^[FCount], (NewCount - FCount) * SizeOf(Pointer), 0);
  FCount := NewCount;
end;

function TFPList.Expand: TFPList;
var
  Delta: Integer;
begin
  if FCount = FCapacity then
  begin
    if FCapacity > 64 then
      Delta := FCapacity div 4
    else if FCapacity > 8 then
      Delta := 16
    else
      Delta := 4;
    SetCapacity(FCapacity + Delta);
  end;
  Result := Self;
end;

function TFPList.Add(Item: Pointer): Integer;
begin
  if FCount = FCapacity then
    Expand;
  FList^[FCount] := Item;
  Result := FCount;
  Inc(FCount);
end;

procedure TFPList.Clear;
begin
  if FList <> nil then
    FreeMem(FList);
  FList := nil;
  FCount := 0;
  FCapacity := 0;
end;

procedure TFPList.Delete(Index: Integer);
begin
  CheckIndex(Index);
  Dec(FCount);
  if Index < FCount then
    System.Move(FList^[Index + 1], FList^[Index], (FCount - Index) * SizeOf(Pointer));
end;

procedure TFPList.Exchange(Index1, Index2: Integer);
var
  Temp: Pointer;
begin
  CheckIndex(Index1);
  CheckIndex(Index2);
  Temp := FList^[Index1];
  FList^[Index1] := FList^[Index2];
  FList^[Index2] := Temp;
end;

function TFPList.Extract(Item: Pointer): Pointer;
var
  I: Integer;
begin
  Result := nil;
  I := IndexOf(Item);
  if I >= 0 then
  begin
    Result := Item;
    Delete(I);
  end;
end;

function TFPList.First: Pointer;
begin
  if FCount = 0 then
    Result := nil
  else
    Result := FList^[0];
end;

function TFPList.Last: Pointer;
begin
  if FCount = 0 then
    Result := nil
  else
    Result := FList^[FCount - 1];
end;

function TFPList.IndexOf(Item: Pointer): Integer;
begin
  Result := 0;
  while (Result < FCount) and (FList^[Result] <> Item) do
    Inc(Result);
  if Result = FCount then
    Result := -1;
end;

procedure TFPList.Insert(Index: Integer; Item: Pointer);
begin
  if (Index < 0) or (Index > FCount) then
    raise ListIndexError(Index);
  if FCount = FCapacity then
    Expand;
  if Index < FCount then
    System.Move(FList^[Index], FList^[Index + 1], (FCount - Index) * SizeOf(Pointer));
  FList^[Index] := Item;
  Inc(FCount);
end;

procedure TFPList.Move(CurIndex, NewIndex: Integer);
var
  Temp: Pointer;
begin
  if CurIndex = NewIndex then
    Exit;
  CheckIndex(CurIndex);
  CheckIndex(NewIndex);
  Temp := FList^[CurIndex];
  Delete(CurIndex);
  Insert(NewIndex, Temp);
end;

procedure TFPList.Pack;
var
  I, J: Integer;
begin
  J := 0;
  for I := 0 to FCount - 1 do
    if FList^[I] <> nil then
    begin
      FList^[J] := FList^[I];
      Inc(J);
    end;
  FCount := J;
end;

function TFPList.Remove(Item: Pointer): Integer;
begin
  Result := IndexOf(Item);
  if Result >= 0 then
    Delete(Result);
end;

procedure ListQuickSort(List: PPointerList; L, R: Integer; Compare: TListSortCompare);
var
  I, J: Integer;
  P, T: Pointer;
begin
  repeat
    I := L;
    J := R;
    P := List^[(L + R) div 2];
    repeat
      while Compare(P, List^[I]) > 0 do
        Inc(I);
      while Compare(P, List^[J]) < 0 do
        Dec(J);
      if I <= J then
      begin
        T := List^[I];
        List^[I] := List^[J];
        List^[J] := T;
        Inc(I);
        Dec(J);
      end;
    until I > J;
    if L < J then
      ListQuickSort(List, L, J, Compare);
    L := I;
  until I >= R;
end;

procedure TFPList.Sort(Compare: TListSortCompare);
begin
  if FCount > 1 then
    ListQuickSort(FList, 0, FCount - 1, Compare);
end;

procedure TFPList.Assign(ListA: TFPList);
var
  I: Integer;
begin
  Clear;
  for I := 0 to ListA.Count - 1 do
    Add(ListA[I]);
end;

{ TList }

constructor TList.Create;
begin
  inherited Create;
  FList := TFPList.Create;
end;

destructor TList.Destroy;
begin
  if FList <> nil then
    Clear;
  FList.Free;
  inherited Destroy;
end;

function TList.Get(Index: Integer): Pointer;
begin
  Result := FList[Index];
end;

procedure TList.Put(Index: Integer; Item: Pointer);
begin
  FList[Index] := Item;
end;

function TList.GetCount: Integer;
begin
  Result := FList.Count;
end;

procedure TList.SetCount(NewCount: Integer);
begin
  FList.Count := NewCount;
end;

function TList.GetCapacity: Integer;
begin
  Result := FList.Capacity;
end;

procedure TList.SetCapacity(NewCapacity: Integer);
begin
  FList.Capacity := NewCapacity;
end;

function TList.Add(Item: Pointer): Integer;
begin
  Result := FList.Add(Item);
end;

procedure TList.Clear;
begin
  FList.Clear;
end;

procedure TList.Delete(Index: Integer);
begin
  FList.Delete(Index);
end;

procedure TList.Exchange(Index1, Index2: Integer);
begin
  FList.Exchange(Index1, Index2);
end;

function TList.Expand: TList;
begin
  FList.Expand;
  Result := Self;
end;

function TList.Extract(Item: Pointer): Pointer;
begin
  Result := FList.Extract(Item);
end;

function TList.First: Pointer;
begin
  Result := FList.First;
end;

function TList.IndexOf(Item: Pointer): Integer;
begin
  Result := FList.IndexOf(Item);
end;

procedure TList.Insert(Index: Integer; Item: Pointer);
begin
  FList.Insert(Index, Item);
end;

function TList.Last: Pointer;
begin
  Result := FList.Last;
end;

procedure TList.Move(CurIndex, NewIndex: Integer);
begin
  FList.Move(CurIndex, NewIndex);
end;

procedure TList.Pack;
begin
  FList.Pack;
end;

function TList.Remove(Item: Pointer): Integer;
begin
  Result := FList.Remove(Item);
end;

procedure TList.Sort(Compare: TListSortCompare);
begin
  FList.Sort(Compare);
end;

{ TPersistent }

procedure TPersistent.AssignTo(Dest: TPersistent);
var
  Name: string;
begin
  if Self = nil then
    Name := 'nil'
  else
    Name := ClassName;
  raise EConvertError.Create('Cannot assign a ' + Name + ' to a ' + Dest.ClassName + '.');
end;

procedure TPersistent.Assign(Source: TPersistent);
begin
  if Source <> nil then
    Source.AssignTo(Self)
  else
    raise EConvertError.Create('Cannot assign a nil to a ' + ClassName + '.');
end;

{ TComponent }

constructor TComponent.Create(AOwner: TComponent);
begin
  inherited Create;
  if AOwner <> nil then
    AOwner.InsertComponent(Self);
end;

destructor TComponent.Destroy;
var
  C: TComponent;
begin
  if FComponents <> nil then
  begin
    while FComponents.Count > 0 do
    begin
      C := TComponent(FComponents.Last);
      RemoveComponent(C);
      C.Free;
    end;
    FComponents.Free;
    FComponents := nil;
  end;
  if FOwner <> nil then
    FOwner.RemoveComponent(Self);
  inherited Destroy;
end;

procedure TComponent.InsertComponent(AComponent: TComponent);
begin
  if FComponents = nil then
    FComponents := TFPList.Create;
  FComponents.Add(AComponent);
  AComponent.FOwner := Self;
end;

procedure TComponent.RemoveComponent(AComponent: TComponent);
begin
  if FComponents <> nil then
    FComponents.Remove(AComponent);
  AComponent.FOwner := nil;
end;

function TComponent.FindComponent(const AName: string): TComponent;
var
  I: Integer;
begin
  Result := nil;
  if FComponents <> nil then
    for I := 0 to FComponents.Count - 1 do
      if CompareText(TComponent(FComponents[I]).Name, AName) = 0 then
        Exit(TComponent(FComponents[I]));
end;

function TComponent.GetComponent(Index: Integer): TComponent;
begin
  if FComponents = nil then
    raise ListIndexError(Index);
  Result := TComponent(FComponents[Index]);
end;

function TComponent.GetComponentCount: Integer;
begin
  if FComponents = nil then
    Result := 0
  else
    Result := FComponents.Count;
end;

{ TStream }

function TStream.GetPosition: Int64;
begin
  Result := Seek(0, soCurrent);
end;

procedure TStream.SetPosition(const Pos: Int64);
begin
  Seek(Pos, soBeginning);
end;

function TStream.GetSize: Int64;
var
  P: Int64;
begin
  P := Seek(0, soCurrent);
  Result := Seek(0, soEnd);
  Seek(P, soBeginning);
end;

procedure TStream.SetSize(const NewSize: Int64);
begin
end;

procedure TStream.ReadBuffer(var Buffer; Count: LongInt);
var
  Done, N: LongInt;
begin
  Done := 0;
  while Done < Count do
  begin
    N := Read(PByte(PtrUInt(@Buffer) + PtrUInt(Done))^, Count - Done);
    if N <= 0 then
      raise EReadError.Create('Stream read error');
    Done := Done + N;
  end;
end;

procedure TStream.WriteBuffer(const Buffer; Count: LongInt);
var
  Done, N: LongInt;
begin
  Done := 0;
  while Done < Count do
  begin
    N := Write(PByte(PtrUInt(@Buffer) + PtrUInt(Done))^, Count - Done);
    if N <= 0 then
      raise EWriteError.Create('Stream write error');
    Done := Done + N;
  end;
end;

function TStream.CopyFrom(Source: TStream; Count: Int64): Int64;
var
  Buffer: array[0..8191] of Byte;
  N: LongInt;
begin
  if Count <= 0 then
  begin
    Source.Position := 0;
    Count := Source.Size;
  end;
  Result := 0;
  while Count > 0 do
  begin
    if Count > SizeOf(Buffer) then
      N := SizeOf(Buffer)
    else
      N := Count;
    Source.ReadBuffer(Buffer, N);
    WriteBuffer(Buffer, N);
    Count := Count - N;
    Result := Result + N;
  end;
end;

function TStream.ReadByte: Byte;
var
  B: Byte;
begin
  ReadBuffer(B, 1);
  Result := B;
end;

procedure TStream.WriteByte(B: Byte);
begin
  WriteBuffer(B, 1);
end;

function TStream.ReadAnsiString: AnsiString;
var
  Len: LongInt;
  S: AnsiString;
begin
  Len := 0;
  ReadBuffer(Len, SizeOf(Len));
  S := '';
  if Len > 0 then
  begin
    SetLength(S, Len);
    ReadBuffer(S[1], Len);
  end;
  Result := S;
end;

procedure TStream.WriteAnsiString(const S: AnsiString);
var
  Len: LongInt;
begin
  Len := Length(S);
  WriteBuffer(Len, SizeOf(Len));
  if Len > 0 then
    WriteBuffer(S[1], Len);
end;

{ THandleStream }

constructor THandleStream.Create(AHandle: THandle);
begin
  inherited Create;
  FHandle := AHandle;
end;

function THandleStream.Read(var Buffer; Count: LongInt): LongInt;
begin
  Result := FileRead(FHandle, Buffer, Count);
  if Result < 0 then
    Result := 0;
end;

function THandleStream.Write(const Buffer; Count: LongInt): LongInt;
begin
  Result := FileWrite(FHandle, Buffer, Count);
  if Result < 0 then
    Result := 0;
end;

function THandleStream.Seek(const Offset: Int64; Origin: TSeekOrigin): Int64;
begin
  Result := FileSeek(FHandle, Offset, Ord(Origin));
end;

procedure THandleStream.SetSize(const NewSize: Int64);
begin
  raise EStreamError.Create('Cannot set the size of a handle stream');
end;

{ TFileStream }

constructor TFileStream.Create(const AFileName: string; Mode: Word);
var
  H: THandle;
begin
  FFileName := AFileName;
  if (Mode and fmCreate) = fmCreate then
  begin
    H := FileCreate(AFileName);
    if H < 0 then
      raise EFCreateError.Create('Unable to create file "' + AFileName + '": ' +
        SysErrorMessage(GetLastOSError));
  end
  else
  begin
    H := FileOpen(AFileName, Mode);
    if H < 0 then
      raise EFOpenError.Create('Unable to open file "' + AFileName + '": ' +
        SysErrorMessage(GetLastOSError));
  end;
  inherited Create(H);
end;

destructor TFileStream.Destroy;
begin
  if FHandle >= 0 then
    FileClose(FHandle);
  inherited Destroy;
end;

{ TCustomMemoryStream }

procedure TCustomMemoryStream.SetPointer(Ptr: Pointer; ASize: PtrInt);
begin
  FMemory := Ptr;
  FSize := ASize;
end;

function TCustomMemoryStream.Read(var Buffer; Count: LongInt): LongInt;
begin
  Result := 0;
  if (FPosition >= 0) and (Count > 0) then
  begin
    Result := FSize - FPosition;
    if Result > Count then
      Result := Count;
    if Result < 0 then
      Result := 0;
    if Result > 0 then
      System.Move(PByte(PtrUInt(FMemory) + PtrUInt(FPosition))^, Buffer, Result);
    FPosition := FPosition + Result;
  end;
end;

function TCustomMemoryStream.Seek(const Offset: Int64; Origin: TSeekOrigin): Int64;
begin
  case Origin of
    soBeginning: FPosition := Offset;
    soCurrent: FPosition := FPosition + Offset;
    soEnd: FPosition := FSize + Offset;
  end;
  Result := FPosition;
end;

procedure TCustomMemoryStream.SaveToStream(Stream: TStream);
begin
  if FSize > 0 then
    Stream.WriteBuffer(FMemory^, FSize);
end;

procedure TCustomMemoryStream.SaveToFile(const FileName: string);
var
  S: TFileStream;
begin
  S := TFileStream.Create(FileName, fmCreate);
  try
    SaveToStream(S);
  finally
    S.Free;
  end;
end;

{ TMemoryStream }

destructor TMemoryStream.Destroy;
begin
  Clear;
  inherited Destroy;
end;

procedure TMemoryStream.SetCapacity(NewCapacity: PtrInt);
begin
  if NewCapacity = FCapacity then
    Exit;
  ReallocMem(FMemory, NewCapacity);
  FCapacity := NewCapacity;
end;

procedure TMemoryStream.SetSize(const NewSize: Int64);
begin
  if NewSize > FCapacity then
    SetCapacity((NewSize + 4095) and not 4095);
  FSize := NewSize;
  if FPosition > FSize then
    FPosition := FSize;
end;

procedure TMemoryStream.Clear;
begin
  if FMemory <> nil then
    FreeMem(FMemory);
  FMemory := nil;
  FCapacity := 0;
  FSize := 0;
  FPosition := 0;
end;

procedure TMemoryStream.LoadFromStream(Stream: TStream);
begin
  Stream.Position := 0;
  SetSize(Stream.Size);
  if FSize > 0 then
    Stream.ReadBuffer(FMemory^, FSize);
  FPosition := 0;
end;

procedure TMemoryStream.LoadFromFile(const FileName: string);
var
  S: TFileStream;
begin
  S := TFileStream.Create(FileName, fmOpenRead);
  try
    LoadFromStream(S);
  finally
    S.Free;
  end;
end;

function TMemoryStream.Write(const Buffer; Count: LongInt): LongInt;
var
  EndPos: PtrInt;
begin
  Result := 0;
  if (FPosition < 0) or (Count <= 0) then
    Exit;
  EndPos := FPosition + Count;
  if EndPos > FSize then
  begin
    if EndPos > FCapacity then
      SetCapacity((EndPos + 4095) and not 4095);
    FSize := EndPos;
  end;
  System.Move(Buffer, PByte(PtrUInt(FMemory) + PtrUInt(FPosition))^, Count);
  FPosition := EndPos;
  Result := Count;
end;

{ TStringStream }

constructor TStringStream.Create(const AString: string);
begin
  inherited Create;
  WriteString(AString);
  FPosition := 0;
end;

function TStringStream.GetDataString: string;
var
  S: string;
begin
  S := '';
  if FSize > 0 then
  begin
    SetLength(S, FSize);
    System.Move(FMemory^, S[1], FSize);
  end;
  Result := S;
end;

function TStringStream.ReadString(Count: LongInt): string;
var
  S: string;
  N: LongInt;
begin
  S := '';
  if Count > FSize - FPosition then
    Count := FSize - FPosition;
  if Count > 0 then
  begin
    SetLength(S, Count);
    N := Read(S[1], Count);
    SetLength(S, N);
  end;
  Result := S;
end;

procedure TStringStream.WriteString(const AString: string);
begin
  if Length(AString) > 0 then
    WriteBuffer(AString[1], Length(AString));
end;

{ TStringsEnumerator }

constructor TStringsEnumerator.Create(AStrings: TStrings);
begin
  inherited Create;
  FStrings := AStrings;
  FPosition := -1;
end;

function TStringsEnumerator.GetCurrent: string;
begin
  Result := FStrings[FPosition];
end;

function TStringsEnumerator.MoveNext: Boolean;
begin
  Inc(FPosition);
  Result := FPosition < FStrings.Count;
end;

{ TStrings }

constructor TStrings.Create;
begin
  inherited Create;
  FDelimiter := ',';
  FQuoteChar := '"';
  FNameValueSeparator := '=';
  FLineBreak := LineEnding;
end;

function TStrings.GetObject(Index: Integer): TObject;
begin
  Result := nil;
end;

procedure TStrings.Put(Index: Integer; const S: string);
var
  O: TObject;
begin
  O := GetObject(Index);
  Delete(Index);
  InsertObject(Index, S, O);
end;

procedure TStrings.PutObject(Index: Integer; AObject: TObject);
begin
end;

function TStrings.Add(const S: string): Integer;
begin
  Result := Count;
  Insert(Result, S);
end;

function TStrings.AddObject(const S: string; AObject: TObject): Integer;
begin
  Result := Add(S);
  PutObject(Result, AObject);
end;

procedure TStrings.Append(const S: string);
begin
  Add(S);
end;

procedure TStrings.AddStrings(TheStrings: TStrings);
var
  I: Integer;
begin
  BeginUpdate;
  try
    for I := 0 to TheStrings.Count - 1 do
      AddObject(TheStrings[I], TheStrings.Objects[I]);
  finally
    EndUpdate;
  end;
end;

procedure TStrings.Assign(Source: TPersistent);
begin
  if Source is TStrings then
  begin
    BeginUpdate;
    try
      Clear;
      FDelimiter := TStrings(Source).FDelimiter;
      FQuoteChar := TStrings(Source).FQuoteChar;
      FNameValueSeparator := TStrings(Source).FNameValueSeparator;
      FStrictDelimiter := TStrings(Source).FStrictDelimiter;
      AddStrings(TStrings(Source));
    finally
      EndUpdate;
    end;
  end
  else
    inherited Assign(Source);
end;

procedure TStrings.SetUpdateState(Updating: Boolean);
begin
end;

procedure TStrings.BeginUpdate;
begin
  if FUpdateCount = 0 then
    SetUpdateState(True);
  Inc(FUpdateCount);
end;

procedure TStrings.EndUpdate;
begin
  if FUpdateCount > 0 then
    Dec(FUpdateCount);
  if FUpdateCount = 0 then
    SetUpdateState(False);
end;

procedure TStrings.InsertObject(Index: Integer; const S: string; AObject: TObject);
begin
  Insert(Index, S);
  PutObject(Index, AObject);
end;

procedure TStrings.Exchange(Index1, Index2: Integer);
var
  S: string;
  O: TObject;
begin
  BeginUpdate;
  try
    S := Strings[Index1];
    O := Objects[Index1];
    Strings[Index1] := Strings[Index2];
    Objects[Index1] := Objects[Index2];
    Strings[Index2] := S;
    Objects[Index2] := O;
  finally
    EndUpdate;
  end;
end;

procedure TStrings.Move(CurIndex, NewIndex: Integer);
var
  S: string;
  O: TObject;
begin
  if CurIndex = NewIndex then
    Exit;
  BeginUpdate;
  try
    S := Strings[CurIndex];
    O := Objects[CurIndex];
    Delete(CurIndex);
    InsertObject(NewIndex, S, O);
  finally
    EndUpdate;
  end;
end;

function TStrings.Equals(Obj: TObject): Boolean;
var
  I: Integer;
  Other: TStrings;
begin
  Result := False;
  if not (Obj is TStrings) then
    Exit;
  Other := TStrings(Obj);
  if Other.Count <> Count then
    Exit;
  for I := 0 to Count - 1 do
    if Strings[I] <> Other[I] then
      Exit;
  Result := True;
end;

function TStrings.DoCompareText(const S1, S2: string): Integer;
begin
  Result := CompareText(S1, S2);
end;

function TStrings.IndexOf(const S: string): Integer;
begin
  Result := 0;
  while (Result < Count) and (DoCompareText(Strings[Result], S) <> 0) do
    Inc(Result);
  if Result = Count then
    Result := -1;
end;

function TStrings.IndexOfObject(AObject: TObject): Integer;
begin
  Result := 0;
  while (Result < Count) and (Objects[Result] <> AObject) do
    Inc(Result);
  if Result = Count then
    Result := -1;
end;

function TStrings.GetName(Index: Integer): string;
var
  S: string;
  P: Integer;
begin
  S := Strings[Index];
  P := Pos(FNameValueSeparator, S);
  if P > 0 then
    Result := Copy(S, 1, P - 1)
  else
    Result := '';
end;

function TStrings.IndexOfName(const Name: string): Integer;
var
  S: string;
  P: Integer;
begin
  for Result := 0 to Count - 1 do
  begin
    S := Strings[Result];
    P := Pos(FNameValueSeparator, S);
    if (P > 0) and (DoCompareText(Copy(S, 1, P - 1), Name) = 0) then
      Exit;
  end;
  Result := -1;
end;

function TStrings.GetValue(const Name: string): string;
var
  I: Integer;
begin
  I := IndexOfName(Name);
  if I < 0 then
    Result := ''
  else
    Result := GetValueFromIndex(I);
end;

procedure TStrings.SetValue(const Name, Value: string);
var
  I: Integer;
begin
  I := IndexOfName(Name);
  if I < 0 then
    Add(Name + FNameValueSeparator + Value)
  else
    Strings[I] := Name + FNameValueSeparator + Value;
end;

function TStrings.GetValueFromIndex(Index: Integer): string;
var
  S: string;
  P: Integer;
begin
  S := Strings[Index];
  P := Pos(FNameValueSeparator, S);
  if P > 0 then
    Result := Copy(S, P + 1, Length(S))
  else
    Result := '';
end;

procedure TStrings.SetValueFromIndex(Index: Integer; const Value: string);
var
  N: string;
begin
  N := GetName(Index);
  Strings[Index] := N + FNameValueSeparator + Value;
end;

function TStrings.GetTextStr: string;
var
  I: Integer;
  R: string;
begin
  R := '';
  for I := 0 to Count - 1 do
    R := R + Strings[I] + FLineBreak;
  Result := R;
end;

procedure TStrings.SetTextStr(const Value: string);
var
  I, Start, Len: Integer;
begin
  BeginUpdate;
  try
    Clear;
    Len := Length(Value);
    I := 1;
    Start := 1;
    while I <= Len do
    begin
      if (Value[I] = #10) or (Value[I] = #13) then
      begin
        Add(Copy(Value, Start, I - Start));
        if (Value[I] = #13) and (I < Len) and (Value[I + 1] = #10) then
          Inc(I);
        Start := I + 1;
      end;
      Inc(I);
    end;
    if Start <= Len then
      Add(Copy(Value, Start, Len - Start + 1));
  finally
    EndUpdate;
  end;
end;

function TStrings.GetDelimitedText: string;
var
  I, J: Integer;
  S, R: string;
  NeedQuotes: Boolean;
begin
  R := '';
  for I := 0 to Count - 1 do
  begin
    S := Strings[I];
    NeedQuotes := False;
    if not FStrictDelimiter then
      for J := 1 to Length(S) do
        if (S[J] <= ' ') or (S[J] = FDelimiter) or (S[J] = FQuoteChar) then
          NeedQuotes := True;
    if FStrictDelimiter then
      for J := 1 to Length(S) do
        if (S[J] = FDelimiter) or (S[J] = FQuoteChar) then
          NeedQuotes := True;
    if NeedQuotes then
      S := AnsiQuotedStr(S, FQuoteChar);
    if I > 0 then
      R := R + FDelimiter;
    R := R + S;
  end;
  if (Count = 1) and (R = '') then
    R := FQuoteChar + FQuoteChar;
  Result := R;
end;

procedure TStrings.SetDelimitedText(const Value: string);
var
  I, Len: Integer;
  S: string;
  Ended: Boolean;
begin
  BeginUpdate;
  try
    Clear;
    Len := Length(Value);
    I := 1;
    if not FStrictDelimiter then
      while (I <= Len) and (Value[I] <= ' ') do
        Inc(I);
    while I <= Len do
    begin
      S := '';
      if Value[I] = FQuoteChar then
      begin
        Inc(I);
        Ended := False;
        while (I <= Len) and not Ended do
        begin
          if Value[I] = FQuoteChar then
          begin
            if (I < Len) and (Value[I + 1] = FQuoteChar) then
            begin
              S := S + FQuoteChar;
              Inc(I, 2);
            end
            else
            begin
              Ended := True;
              Inc(I);
            end;
          end
          else
          begin
            S := S + Value[I];
            Inc(I);
          end;
        end;
      end
      else
        while (I <= Len) and (Value[I] <> FDelimiter)
          and (FStrictDelimiter or (Value[I] > ' ')) do
        begin
          S := S + Value[I];
          Inc(I);
        end;
      Add(S);
      if not FStrictDelimiter then
        while (I <= Len) and (Value[I] <= ' ') do
          Inc(I);
      if (I <= Len) and (Value[I] = FDelimiter) then
      begin
        Inc(I);
        if not FStrictDelimiter then
          while (I <= Len) and (Value[I] <= ' ') do
            Inc(I);
        if I > Len then
          Add('');
      end;
    end;
  finally
    EndUpdate;
  end;
end;

function TStrings.GetCommaText: string;
var
  OldDelimiter, OldQuote: Char;
  OldStrict: Boolean;
begin
  OldDelimiter := FDelimiter;
  OldQuote := FQuoteChar;
  OldStrict := FStrictDelimiter;
  FDelimiter := ',';
  FQuoteChar := '"';
  FStrictDelimiter := False;
  try
    Result := GetDelimitedText;
  finally
    FDelimiter := OldDelimiter;
    FQuoteChar := OldQuote;
    FStrictDelimiter := OldStrict;
  end;
end;

procedure TStrings.SetCommaText(const Value: string);
var
  OldDelimiter, OldQuote: Char;
  OldStrict: Boolean;
begin
  OldDelimiter := FDelimiter;
  OldQuote := FQuoteChar;
  OldStrict := FStrictDelimiter;
  FDelimiter := ',';
  FQuoteChar := '"';
  FStrictDelimiter := False;
  try
    SetDelimitedText(Value);
  finally
    FDelimiter := OldDelimiter;
    FQuoteChar := OldQuote;
    FStrictDelimiter := OldStrict;
  end;
end;

procedure TStrings.LoadFromStream(Stream: TStream);
var
  S: string;
  N: Int64;
begin
  N := Stream.Size - Stream.Position;
  S := '';
  if N > 0 then
  begin
    SetLength(S, N);
    Stream.ReadBuffer(S[1], N);
  end;
  SetTextStr(S);
end;

procedure TStrings.LoadFromFile(const FileName: string);
var
  Stream: TFileStream;
begin
  Stream := TFileStream.Create(FileName, fmOpenRead);
  try
    LoadFromStream(Stream);
  finally
    Stream.Free;
  end;
end;

procedure TStrings.SaveToStream(Stream: TStream);
var
  S: string;
begin
  S := GetTextStr;
  if Length(S) > 0 then
    Stream.WriteBuffer(S[1], Length(S));
end;

procedure TStrings.SaveToFile(const FileName: string);
var
  Stream: TFileStream;
begin
  Stream := TFileStream.Create(FileName, fmCreate);
  try
    SaveToStream(Stream);
  finally
    Stream.Free;
  end;
end;

function TStrings.GetEnumerator: TStringsEnumerator;
begin
  Result := TStringsEnumerator.Create(Self);
end;

{ TStringList }

destructor TStringList.Destroy;
begin
  FOnChange := nil;
  FOnChanging := nil;
  Clear;
  inherited Destroy;
end;

procedure TStringList.CheckIndex(Index: Integer);
begin
  if (Index < 0) or (Index >= FCount) then
    raise EStringListError.Create('List index (' + IntToStr(Index) + ') out of bounds');
end;

procedure TStringList.Changed;
begin
  if (FUpdateCount = 0) and Assigned(FOnChange) then
    FOnChange(Self);
end;

procedure TStringList.Changing;
begin
  if (FUpdateCount = 0) and Assigned(FOnChanging) then
    FOnChanging(Self);
end;

procedure TStringList.SetUpdateState(Updating: Boolean);
begin
  if Updating then
    Changing
  else
    Changed;
end;

procedure TStringList.SetCapacity(NewCapacity: Integer);
begin
  if NewCapacity < FCount then
    NewCapacity := FCount;
  if NewCapacity = FCapacity then
    Exit;
  ReallocMem(FList, NewCapacity * SizeOf(TStringItem));
  if NewCapacity > FCapacity then
    FillChar(FList^[FCapacity], (NewCapacity - FCapacity) * SizeOf(TStringItem), 0);
  FCapacity := NewCapacity;
end;

procedure TStringList.Grow;
var
  Delta: Integer;
begin
  if FCapacity > 64 then
    Delta := FCapacity div 4
  else if FCapacity > 8 then
    Delta := 16
  else
    Delta := 4;
  SetCapacity(FCapacity + Delta);
end;

function TStringList.Get(Index: Integer): string;
begin
  CheckIndex(Index);
  Result := FList^[Index].FString;
end;

function TStringList.GetCount: Integer;
begin
  Result := FCount;
end;

function TStringList.GetObject(Index: Integer): TObject;
begin
  CheckIndex(Index);
  Result := FList^[Index].FObject;
end;

procedure TStringList.Put(Index: Integer; const S: string);
begin
  if FSorted then
    raise EStringListError.Create('Operation not allowed on sorted list');
  CheckIndex(Index);
  Changing;
  FList^[Index].FString := S;
  Changed;
end;

procedure TStringList.PutObject(Index: Integer; AObject: TObject);
begin
  CheckIndex(Index);
  Changing;
  FList^[Index].FObject := AObject;
  Changed;
end;

procedure TStringList.InsertItem(Index: Integer; const S: string; O: TObject);
begin
  Changing;
  if FCount = FCapacity then
    Grow;
  if Index < FCount then
  begin
    System.Move(FList^[Index], FList^[Index + 1], (FCount - Index) * SizeOf(TStringItem));
    { The moved-from slot's string is now owned by the slot after it. }
    FillChar(FList^[Index], SizeOf(TStringItem), 0);
  end;
  FList^[Index].FString := S;
  FList^[Index].FObject := O;
  Inc(FCount);
  Changed;
end;

function TStringList.DoCompareText(const S1, S2: string): Integer;
begin
  if FCaseSensitive then
    Result := AnsiCompareStr(S1, S2)
  else
    Result := AnsiCompareText(S1, S2);
end;

function TStringList.Find(const S: string; var Index: Integer): Boolean;
var
  L, H, I, C: Integer;
begin
  Result := False;
  L := 0;
  H := FCount - 1;
  while L <= H do
  begin
    I := (L + H) div 2;
    C := DoCompareText(FList^[I].FString, S);
    if C < 0 then
      L := I + 1
    else
    begin
      H := I - 1;
      if C = 0 then
      begin
        Result := True;
        if FDuplicates <> dupAccept then
          L := I;
      end;
    end;
  end;
  Index := L;
end;

function TStringList.IndexOf(const S: string): Integer;
begin
  if not FSorted then
    Result := inherited IndexOf(S)
  else if not Find(S, Result) then
    Result := -1;
end;

function TStringList.Add(const S: string): Integer;
begin
  Result := AddObject(S, nil);
end;

function TStringList.AddObject(const S: string; AObject: TObject): Integer;
begin
  if not FSorted then
    Result := FCount
  else if Find(S, Result) then
    case FDuplicates of
      dupIgnore: Exit;
      dupError: raise EStringListError.Create('String list does not allow duplicates');
    end;
  InsertItem(Result, S, AObject);
end;

procedure TStringList.Insert(Index: Integer; const S: string);
begin
  if FSorted then
    raise EStringListError.Create('Operation not allowed on sorted list');
  if (Index < 0) or (Index > FCount) then
    raise EStringListError.Create('List index (' + IntToStr(Index) + ') out of bounds');
  InsertItem(Index, S, nil);
end;

procedure TStringList.Clear;
var
  I: Integer;
begin
  if FCount = 0 then
    Exit;
  Changing;
  for I := 0 to FCount - 1 do
  begin
    FList^[I].FString := '';
    if FOwnsObjects then
      FreeAndNil(FList^[I].FObject);
  end;
  FCount := 0;
  SetCapacity(0);
  Changed;
end;

procedure TStringList.Delete(Index: Integer);
begin
  CheckIndex(Index);
  Changing;
  FList^[Index].FString := '';
  if FOwnsObjects then
    FreeAndNil(FList^[Index].FObject);
  Dec(FCount);
  if Index < FCount then
  begin
    System.Move(FList^[Index + 1], FList^[Index], (FCount - Index) * SizeOf(TStringItem));
    FillChar(FList^[FCount], SizeOf(TStringItem), 0);
  end;
  Changed;
end;

procedure TStringList.Exchange(Index1, Index2: Integer);
var
  Temp: TStringItem;
begin
  CheckIndex(Index1);
  CheckIndex(Index2);
  Changing;
  Temp := FList^[Index1];
  FList^[Index1] := FList^[Index2];
  FList^[Index2] := Temp;
  Changed;
end;

procedure TStringList.QuickSort(L, R: Integer; Compare: TStringListSortCompare);
var
  I, J, P: Integer;
  Temp: TStringItem;
begin
  repeat
    I := L;
    J := R;
    P := (L + R) div 2;
    repeat
      while Compare(Self, I, P) < 0 do
        Inc(I);
      while Compare(Self, J, P) > 0 do
        Dec(J);
      if I <= J then
      begin
        if I <> J then
        begin
          Temp := FList^[I];
          FList^[I] := FList^[J];
          FList^[J] := Temp;
        end;
        if P = I then
          P := J
        else if P = J then
          P := I;
        Inc(I);
        Dec(J);
      end;
    until I > J;
    if L < J then
      QuickSort(L, J, Compare);
    L := I;
  until I >= R;
end;

function StringListCompareStrings(List: TStringList; Index1, Index2: Integer): Integer;
begin
  Result := List.DoCompareText(List.FList^[Index1].FString, List.FList^[Index2].FString);
end;

procedure TStringList.CustomSort(CompareFn: TStringListSortCompare);
begin
  if FCount > 1 then
  begin
    Changing;
    QuickSort(0, FCount - 1, CompareFn);
    Changed;
  end;
end;

procedure TStringList.Sort;
begin
  CustomSort(@StringListCompareStrings);
end;

procedure TStringList.SetSorted(Value: Boolean);
begin
  if FSorted <> Value then
  begin
    if Value then
      Sort;
    FSorted := Value;
  end;
end;

procedure TStringList.SetCaseSensitive(Value: Boolean);
begin
  if FCaseSensitive <> Value then
  begin
    FCaseSensitive := Value;
    if FSorted then
      Sort;
  end;
end;

end.
