{ Bruto Pascal runtime: the core of FPC's Generics.Collections: TList<T>,
  TObjectList<T>, TStack<T>, TQueue<T>, TDictionary<TKey, TValue> (a hash
  table; keys are strings, integers, characters, pointers or classes)
  and TPair<TKey, TValue>, with Generics.Defaults' comparers
  (IComparer<T> for TList, IEqualityComparer<TKey> for TDictionary),
  and TArrayHelper<T>'s Sort and BinarySearch. Without a comparer, values
  compare as TComparer<T>.Default does. TDictionary's Keys and Values are
  snapshots of what it held when last asked for. }
unit Generics.Collections;

{$mode objfpc}{$H+}

interface

uses SysUtils, Classes, Generics.Defaults;

type
  generic TArray<T> = array of T;

  TBinarySearchResult = record
    FoundIndex, CandidateIndex: SizeInt;
    CompareResult: SizeInt;
  end;

  { FPC's TArrayHelper<T>: sorting and searching an array. }
  generic TArrayHelper<T> = class
  protected
    class procedure QuickSort(var AValues: array of T; ALeft, ARight: SizeInt;
      const AComparer: specialize IComparer<T>);
  public
    class procedure Sort(var AValues: array of T); overload;
    class procedure Sort(var AValues: array of T;
      const AComparer: specialize IComparer<T>); overload;
    class procedure Sort(var AValues: array of T;
      const AComparer: specialize IComparer<T>; AIndex, ACount: SizeInt); overload;
    class function BinarySearch(const AValues: array of T; const AItem: T;
      out AFoundIndex: SizeInt; const AComparer: specialize IComparer<T>;
      AIndex, ACount: SizeInt): Boolean; overload;
    class function BinarySearch(const AValues: array of T; const AItem: T;
      out AFoundIndex: SizeInt; const AComparer: specialize IComparer<T>): Boolean; overload;
    class function BinarySearch(const AValues: array of T; const AItem: T;
      out AFoundIndex: SizeInt): Boolean; overload;
    class function BinarySearch(const AValues: array of T; const AItem: T;
      out ASearchResult: TBinarySearchResult; const AComparer: specialize IComparer<T>;
      AIndex, ACount: SizeInt): Boolean; overload;
    class function BinarySearch(const AValues: array of T; const AItem: T;
      out ASearchResult: TBinarySearchResult; const AComparer: specialize IComparer<T>): Boolean; overload;
    class function BinarySearch(const AValues: array of T; const AItem: T;
      out ASearchResult: TBinarySearchResult): Boolean; overload;
  end;

  generic TPair<TKey, TValue> = record
    Key: TKey;
    Value: TValue;
  end;

  generic TListEnumerator<T> = class
  private
    FItems: array of T;
    FCount: SizeInt;
    FPosition: SizeInt;
  public
    constructor Create(const AItems: array of T; ACount: SizeInt);
    function GetCurrent: T;
    function MoveNext: Boolean;
    property Current: T read GetCurrent;
  end;

  generic TList<T> = class
  private
    FItems: array of T;
    FCount: SizeInt;
    FComparer: specialize IComparer<T>;
    function GetItem(Index: SizeInt): T;
    procedure SetItem(Index: SizeInt; const Value: T);
    function GetCapacity: SizeInt;
    procedure SetCapacity(Value: SizeInt);
    procedure SetCount(Value: SizeInt);
    procedure CheckIndex(Index: SizeInt);
    function DoCompare(const A, B: T): Integer;
  protected
    procedure Notify(const Item: T; Removed: Boolean); virtual;
  public
    constructor Create; overload;
    constructor Create(const AComparer: specialize IComparer<T>); overload;
    destructor Destroy; override;
    function Add(const Value: T): SizeInt;
    procedure AddRange(const Values: array of T);
    procedure Insert(Index: SizeInt; const Value: T);
    procedure Delete(Index: SizeInt);
    procedure DeleteRange(AIndex, ACount: SizeInt);
    function Remove(const Value: T): SizeInt;
    function Extract(const Value: T): T;
    procedure Exchange(Index1, Index2: SizeInt);
    procedure Move(CurIndex, NewIndex: SizeInt);
    function First: T;
    function Last: T;
    procedure Clear;
    function Contains(const Value: T): Boolean;
    function IndexOf(const Value: T): SizeInt;
    function LastIndexOf(const Value: T): SizeInt;
    procedure Reverse;
    procedure Sort; overload;
    procedure Sort(const AComparer: specialize IComparer<T>); overload;
    function BinarySearch(const Item: T; out FoundIndex: SizeInt): Boolean; overload;
    function BinarySearch(const Item: T; out FoundIndex: SizeInt;
      const AComparer: specialize IComparer<T>): Boolean; overload;
    function ToArray: specialize TArray<T>;
    function GetEnumerator: specialize TListEnumerator<T>;
    property Count: SizeInt read FCount write SetCount;
    property Capacity: SizeInt read GetCapacity write SetCapacity;
    property Items[Index: SizeInt]: T read GetItem write SetItem; default;
  end;

  generic TObjectList<T> = class(specialize TList<T>)
  private
    FOwnsObjects: Boolean;
  protected
    procedure Notify(const Item: T; Removed: Boolean); override;
  public
    constructor Create(AOwnsObjects: Boolean = True); overload;
    constructor Create(const AComparer: specialize IComparer<T>;
      AOwnsObjects: Boolean = True); overload;
    property OwnsObjects: Boolean read FOwnsObjects write FOwnsObjects;
  end;

  generic TStack<T> = class
  private
    FItems: array of T;
    FCount: SizeInt;
  public
    procedure Push(const Value: T);
    function Pop: T;
    function Peek: T;
    function Extract: T;
    procedure Clear;
    function ToArray: specialize TArray<T>;
    property Count: SizeInt read FCount;
  end;

  generic TQueue<T> = class
  private
    FItems: array of T;
    FHead, FCount: SizeInt;
  public
    procedure Enqueue(const Value: T);
    function Dequeue: T;
    function Peek: T;
    function Extract: T;
    procedure Clear;
    function ToArray: specialize TArray<T>;
    property Count: SizeInt read FCount;
  end;

  { A dictionary's keys or values: what it held when last asked for. }
  generic TCollectionSnapshot<T> = class
  private
    FItems: array of T;
    function GetCount: SizeInt;
  public
    function ToArray: specialize TArray<T>;
    function GetEnumerator: specialize TListEnumerator<T>;
    property Count: SizeInt read GetCount;
  end;

  generic TDictionaryEnumerator<TKey, TValue> = class
  private
    FPairs: array of specialize TPair<TKey, TValue>;
    FPosition: SizeInt;
  public
    constructor Create(const AKeys: array of TKey; const AValues: array of TValue);
    function GetCurrent: specialize TPair<TKey, TValue>;
    function MoveNext: Boolean;
    property Current: specialize TPair<TKey, TValue> read GetCurrent;
  end;

  generic TDictionary<TKey, TValue> = class
  private
    FKeys: array of TKey;
    FValues: array of TValue;
    FUsed: array of Boolean;
    FCount: SizeInt;
    FComparer: specialize IEqualityComparer<TKey>;
    FKeyCollection: specialize TCollectionSnapshot<TKey>;
    FValueCollection: specialize TCollectionSnapshot<TValue>;
    function Slot(const Key: TKey): SizeInt;
    function HashOf(const Key: TKey): QWord;
    procedure Grow;
    function GetItem(const Key: TKey): TValue;
    procedure SetItem(const Key: TKey; const Value: TValue);
    function KeyArray: specialize TArray<TKey>;
    function ValueArray: specialize TArray<TValue>;
    function GetKeys: specialize TCollectionSnapshot<TKey>;
    function GetValues: specialize TCollectionSnapshot<TValue>;
  public
    constructor Create; overload;
    constructor Create(ACapacity: SizeInt); overload;
    constructor Create(const AComparer: specialize IEqualityComparer<TKey>); overload;
    constructor Create(ACapacity: SizeInt;
      const AComparer: specialize IEqualityComparer<TKey>); overload;
    destructor Destroy; override;
    procedure Add(const Key: TKey; const Value: TValue);
    function TryAdd(const Key: TKey; const Value: TValue): Boolean;
    procedure AddOrSetValue(const Key: TKey; const Value: TValue);
    procedure Remove(const Key: TKey);
    procedure Clear;
    function TryGetValue(const Key: TKey; out Value: TValue): Boolean;
    function ContainsKey(const Key: TKey): Boolean;
    function ContainsValue(const Value: TValue): Boolean;
    function ToArray: specialize TArray<specialize TPair<TKey, TValue>>;
    function GetEnumerator: specialize TDictionaryEnumerator<TKey, TValue>;
    property Count: SizeInt read FCount;
    property Items[const Key: TKey]: TValue read GetItem write SetItem; default;
    property Keys: specialize TCollectionSnapshot<TKey> read GetKeys;
    property Values: specialize TCollectionSnapshot<TValue> read GetValues;
  end;

{ The hash of a key, for TDictionary. }
function BrutoHashOf(const S: AnsiString): QWord; overload;
function BrutoHashOf(const S: UnicodeString): QWord; overload;
function BrutoHashOf(V: Int64): QWord; overload;
function BrutoHashOf(V: QWord): QWord; overload;
function BrutoHashOf(V: Char): QWord; overload;
function BrutoHashOf(V: WideChar): QWord; overload;
function BrutoHashOf(V: Pointer): QWord; overload;
function BrutoHashOf(V: Double): QWord; overload;

implementation

function BrutoHashOf(const S: AnsiString): QWord;
var
  I: SizeInt;
begin
  Result := QWord($CBF29CE484222325);
  for I := 1 to Length(S) do
    Result := (Result xor Ord(S[I])) * QWord($100000001B3);
end;

function BrutoHashOf(const S: UnicodeString): QWord;
var
  I: SizeInt;
begin
  Result := QWord($CBF29CE484222325);
  for I := 1 to Length(S) do
    Result := (Result xor Ord(S[I])) * QWord($100000001B3);
end;

function BrutoHashOf(V: Int64): QWord;
begin
  Result := QWord(V) * QWord($9E3779B97F4A7C15);
  Result := Result xor (Result shr 29);
end;

function BrutoHashOf(V: QWord): QWord;
begin
  Result := BrutoHashOf(Int64(V));
end;

function BrutoHashOf(V: Char): QWord;
begin
  Result := BrutoHashOf(Int64(Ord(V)));
end;

function BrutoHashOf(V: WideChar): QWord;
begin
  Result := BrutoHashOf(Int64(Ord(V)));
end;

function BrutoHashOf(V: Pointer): QWord;
begin
  Result := BrutoHashOf(Int64(PtrUInt(V)));
end;

function BrutoHashOf(V: Double): QWord;
begin
  Result := BrutoHashOf(PInt64(@V)^);
end;

function ArgumentOutOfRange: EArgumentOutOfRangeException;
begin
  Result := EArgumentOutOfRangeException.Create('Argument out of range');
end;

{ TArrayHelper }

class procedure TArrayHelper.QuickSort(var AValues: array of T; ALeft, ARight: SizeInt;
  const AComparer: specialize IComparer<T>);
var
  I, J: SizeInt;
  P, Q: T;
begin
  if ((ARight - ALeft) <= 0) or (Length(AValues) = 0) then
    Exit;
  repeat
    I := ALeft;
    J := ARight;
    P := AValues[ALeft + (ARight - ALeft) shr 1];
    repeat
      while AComparer.Compare(AValues[I], P) < 0 do
        Inc(I);
      while AComparer.Compare(AValues[J], P) > 0 do
        Dec(J);
      if I <= J then
      begin
        if I <> J then
        begin
          Q := AValues[I];
          AValues[I] := AValues[J];
          AValues[J] := Q;
        end;
        Inc(I);
        Dec(J);
      end;
    until I > J;
    { The smaller part recursively, the bigger one in the loop. }
    if J - ALeft < ARight - I then
    begin
      if ALeft < J then
        QuickSort(AValues, ALeft, J, AComparer);
      ALeft := I;
    end
    else
    begin
      if I < ARight then
        QuickSort(AValues, I, ARight, AComparer);
      ARight := J;
    end;
  until ALeft >= ARight;
end;

class procedure TArrayHelper.Sort(var AValues: array of T);
begin
  QuickSort(AValues, Low(AValues), High(AValues), specialize TComparer<T>.Default);
end;

class procedure TArrayHelper.Sort(var AValues: array of T;
  const AComparer: specialize IComparer<T>);
begin
  QuickSort(AValues, Low(AValues), High(AValues), AComparer);
end;

class procedure TArrayHelper.Sort(var AValues: array of T;
  const AComparer: specialize IComparer<T>; AIndex, ACount: SizeInt);
begin
  if ACount <= 1 then
    Exit;
  QuickSort(AValues, AIndex, AIndex + ACount - 1, AComparer);
end;

class function TArrayHelper.BinarySearch(const AValues: array of T; const AItem: T;
  out ASearchResult: TBinarySearchResult; const AComparer: specialize IComparer<T>;
  AIndex, ACount: SizeInt): Boolean;
var
  IMin, IMax, IMid: SizeInt;
begin
  IMin := AIndex;
  IMax := AIndex + ACount - 1;
  while IMin < IMax do
  begin
    IMid := IMin + ((IMax - IMin) shr 1);
    ASearchResult.CompareResult := AComparer.Compare(AValues[IMid], AItem);
    if ASearchResult.CompareResult < 0 then
      IMin := IMid + 1
    else
    begin
      IMax := IMid;
      if ASearchResult.CompareResult = 0 then
      begin
        ASearchResult.FoundIndex := IMid;
        ASearchResult.CandidateIndex := IMid;
        Exit(True);
      end;
    end;
  end;
  if IMax = IMin then
  begin
    ASearchResult.CompareResult := AComparer.Compare(AValues[IMin], AItem);
    ASearchResult.CandidateIndex := IMin;
    if ASearchResult.CompareResult = 0 then
    begin
      ASearchResult.FoundIndex := IMin;
      Exit(True);
    end;
    ASearchResult.FoundIndex := -1;
    Exit(False);
  end;
  ASearchResult.CompareResult := 0;
  ASearchResult.FoundIndex := -1;
  ASearchResult.CandidateIndex := -1;
  Result := False;
end;

class function TArrayHelper.BinarySearch(const AValues: array of T; const AItem: T;
  out ASearchResult: TBinarySearchResult; const AComparer: specialize IComparer<T>): Boolean;
begin
  Result := BinarySearch(AValues, AItem, ASearchResult, AComparer, Low(AValues), Length(AValues));
end;

class function TArrayHelper.BinarySearch(const AValues: array of T; const AItem: T;
  out ASearchResult: TBinarySearchResult): Boolean;
begin
  Result := BinarySearch(AValues, AItem, ASearchResult, specialize TComparer<T>.Default,
    Low(AValues), Length(AValues));
end;

class function TArrayHelper.BinarySearch(const AValues: array of T; const AItem: T;
  out AFoundIndex: SizeInt; const AComparer: specialize IComparer<T>;
  AIndex, ACount: SizeInt): Boolean;
var
  IMin, IMax, IMid, LCompare: SizeInt;
begin
  AFoundIndex := -1;
  if ACount <= 0 then
    Exit(False);
  IMin := AIndex;
  IMax := AIndex + ACount - 1;
  while IMin < IMax do
  begin
    IMid := IMin + ((IMax - IMin) shr 1);
    LCompare := AComparer.Compare(AValues[IMid], AItem);
    if LCompare < 0 then
      IMin := IMid + 1
    else
    begin
      IMax := IMid;
      if LCompare = 0 then
      begin
        AFoundIndex := IMid;
        Exit(True);
      end;
    end;
  end;
  LCompare := AComparer.Compare(AValues[IMin], AItem);
  if (IMax = IMin) and (LCompare = 0) then
  begin
    AFoundIndex := IMin;
    Exit(True);
  end;
  Result := False;
end;

class function TArrayHelper.BinarySearch(const AValues: array of T; const AItem: T;
  out AFoundIndex: SizeInt; const AComparer: specialize IComparer<T>): Boolean;
begin
  Result := BinarySearch(AValues, AItem, AFoundIndex, AComparer, Low(AValues), Length(AValues));
end;

class function TArrayHelper.BinarySearch(const AValues: array of T; const AItem: T;
  out AFoundIndex: SizeInt): Boolean;
begin
  Result := BinarySearch(AValues, AItem, AFoundIndex, specialize TComparer<T>.Default,
    Low(AValues), Length(AValues));
end;

{ TListEnumerator }

constructor TListEnumerator.Create(const AItems: array of T; ACount: SizeInt);
var
  I: SizeInt;
begin
  inherited Create;
  SetLength(FItems, ACount);
  for I := 0 to ACount - 1 do
    FItems[I] := AItems[I];
  FCount := ACount;
  FPosition := -1;
end;

function TListEnumerator.GetCurrent: T;
begin
  Result := FItems[FPosition];
end;

function TListEnumerator.MoveNext: Boolean;
begin
  Inc(FPosition);
  Result := FPosition < FCount;
end;

{ TList }

destructor TList.Destroy;
begin
  Clear;
  inherited Destroy;
end;

procedure TList.Notify(const Item: T; Removed: Boolean);
begin
end;

procedure TList.CheckIndex(Index: SizeInt);
begin
  if (Index < 0) or (Index >= FCount) then
    raise ArgumentOutOfRange;
end;

function TList.GetItem(Index: SizeInt): T;
begin
  CheckIndex(Index);
  Result := FItems[Index];
end;

procedure TList.SetItem(Index: SizeInt; const Value: T);
begin
  CheckIndex(Index);
  FItems[Index] := Value;
end;

function TList.GetCapacity: SizeInt;
begin
  Result := Length(FItems);
end;

procedure TList.SetCapacity(Value: SizeInt);
begin
  if Value < FCount then
    Value := FCount;
  SetLength(FItems, Value);
end;

procedure TList.SetCount(Value: SizeInt);
begin
  while FCount > Value do
    Delete(FCount - 1);
  if Value > Length(FItems) then
    SetLength(FItems, Value);
  FCount := Value;
end;

function TList.Add(const Value: T): SizeInt;
begin
  if FCount = Length(FItems) then
    SetLength(FItems, FCount * 2 + 4);
  FItems[FCount] := Value;
  Result := FCount;
  Inc(FCount);
  Notify(Value, False);
end;

procedure TList.AddRange(const Values: array of T);
var
  I: SizeInt;
begin
  for I := 0 to High(Values) do
    Add(Values[I]);
end;

procedure TList.Insert(Index: SizeInt; const Value: T);
var
  I: SizeInt;
begin
  if (Index < 0) or (Index > FCount) then
    raise ArgumentOutOfRange;
  if FCount = Length(FItems) then
    SetLength(FItems, FCount * 2 + 4);
  for I := FCount downto Index + 1 do
    FItems[I] := FItems[I - 1];
  FItems[Index] := Value;
  Inc(FCount);
  Notify(Value, False);
end;

procedure TList.Delete(Index: SizeInt);
var
  I: SizeInt;
  Item: T;
begin
  CheckIndex(Index);
  Item := FItems[Index];
  for I := Index to FCount - 2 do
    FItems[I] := FItems[I + 1];
  Dec(FCount);
  FItems[FCount] := Default(T);
  Notify(Item, True);
end;

procedure TList.DeleteRange(AIndex, ACount: SizeInt);
var
  I: SizeInt;
begin
  if (AIndex < 0) or (ACount < 0) or (AIndex + ACount > FCount) then
    raise ArgumentOutOfRange;
  for I := 1 to ACount do
    Delete(AIndex);
end;

function TList.Remove(const Value: T): SizeInt;
begin
  Result := IndexOf(Value);
  if Result >= 0 then
    Delete(Result);
end;

function TList.Extract(const Value: T): T;
var
  I, J: SizeInt;
begin
  Result := Default(T);
  I := IndexOf(Value);
  if I < 0 then
    Exit;
  Result := FItems[I];
  for J := I to FCount - 2 do
    FItems[J] := FItems[J + 1];
  Dec(FCount);
  FItems[FCount] := Default(T);
end;

procedure TList.Exchange(Index1, Index2: SizeInt);
var
  Temp: T;
begin
  CheckIndex(Index1);
  CheckIndex(Index2);
  Temp := FItems[Index1];
  FItems[Index1] := FItems[Index2];
  FItems[Index2] := Temp;
end;

procedure TList.Move(CurIndex, NewIndex: SizeInt);
var
  Temp: T;
  I: SizeInt;
begin
  CheckIndex(CurIndex);
  CheckIndex(NewIndex);
  Temp := FItems[CurIndex];
  if CurIndex < NewIndex then
    for I := CurIndex to NewIndex - 1 do
      FItems[I] := FItems[I + 1]
  else
    for I := CurIndex downto NewIndex + 1 do
      FItems[I] := FItems[I - 1];
  FItems[NewIndex] := Temp;
end;

function TList.First: T;
begin
  Result := GetItem(0);
end;

function TList.Last: T;
begin
  Result := GetItem(FCount - 1);
end;

procedure TList.Clear;
begin
  while FCount > 0 do
    Delete(FCount - 1);
  SetLength(FItems, 0);
end;

function TList.Contains(const Value: T): Boolean;
begin
  Result := IndexOf(Value) >= 0;
end;

function TList.IndexOf(const Value: T): SizeInt;
begin
  for Result := 0 to FCount - 1 do
    if DoCompare(Value, FItems[Result]) = 0 then
      Exit;
  Result := -1;
end;

function TList.LastIndexOf(const Value: T): SizeInt;
begin
  for Result := FCount - 1 downto 0 do
    if DoCompare(Value, FItems[Result]) = 0 then
      Exit;
  Result := -1;
end;

procedure TList.Reverse;
var
  I: SizeInt;
  Temp: T;
begin
  for I := 0 to FCount div 2 - 1 do
  begin
    Temp := FItems[I];
    FItems[I] := FItems[FCount - 1 - I];
    FItems[FCount - 1 - I] := Temp;
  end;
end;

function TList.DoCompare(const A, B: T): Integer;
begin
  if FComparer = nil then
    Result := BrutoCompare(A, B)
  else
    Result := FComparer.Compare(A, B);
end;

constructor TList.Create;
begin
  inherited Create;
end;

constructor TList.Create(const AComparer: specialize IComparer<T>);
begin
  inherited Create;
  FComparer := AComparer;
end;

procedure TList.Sort;
begin
  if FComparer = nil then
    FComparer := specialize TComparer<T>.Default;
  specialize TArrayHelper<T>.Sort(FItems, FComparer, 0, FCount);
end;

procedure TList.Sort(const AComparer: specialize IComparer<T>);
begin
  specialize TArrayHelper<T>.Sort(FItems, AComparer, 0, FCount);
end;

function TList.BinarySearch(const Item: T; out FoundIndex: SizeInt): Boolean;
begin
  if FComparer = nil then
    FComparer := specialize TComparer<T>.Default;
  Result := specialize TArrayHelper<T>.BinarySearch(FItems, Item, FoundIndex, FComparer, 0, FCount);
end;

function TList.BinarySearch(const Item: T; out FoundIndex: SizeInt;
  const AComparer: specialize IComparer<T>): Boolean;
begin
  Result := specialize TArrayHelper<T>.BinarySearch(FItems, Item, FoundIndex, AComparer, 0, FCount);
end;

function TList.ToArray: specialize TArray<T>;
var
  I: SizeInt;
begin
  SetLength(Result, FCount);
  for I := 0 to FCount - 1 do
    Result[I] := FItems[I];
end;

function TList.GetEnumerator: specialize TListEnumerator<T>;
begin
  Result := specialize TListEnumerator<T>.Create(FItems, FCount);
end;

{ TObjectList }

constructor TObjectList.Create(AOwnsObjects: Boolean);
begin
  inherited Create;
  FOwnsObjects := AOwnsObjects;
end;

constructor TObjectList.Create(const AComparer: specialize IComparer<T>;
  AOwnsObjects: Boolean);
begin
  inherited Create(AComparer);
  FOwnsObjects := AOwnsObjects;
end;

procedure TObjectList.Notify(const Item: T; Removed: Boolean);
begin
  if Removed and FOwnsObjects then
    TObject(Item).Free;
end;

{ TStack }

procedure TStack.Push(const Value: T);
begin
  if FCount = Length(FItems) then
    SetLength(FItems, FCount * 2 + 4);
  FItems[FCount] := Value;
  Inc(FCount);
end;

function TStack.Pop: T;
begin
  if FCount = 0 then
    raise EListError.Create('Stack is empty');
  Dec(FCount);
  Result := FItems[FCount];
  FItems[FCount] := Default(T);
end;

function TStack.Peek: T;
begin
  if FCount = 0 then
    raise EListError.Create('Stack is empty');
  Result := FItems[FCount - 1];
end;

function TStack.Extract: T;
begin
  Result := Pop;
end;

procedure TStack.Clear;
begin
  SetLength(FItems, 0);
  FCount := 0;
end;

function TStack.ToArray: specialize TArray<T>;
var
  I: SizeInt;
begin
  SetLength(Result, FCount);
  for I := 0 to FCount - 1 do
    Result[I] := FItems[I];
end;

{ TQueue }

procedure TQueue.Enqueue(const Value: T);
var
  I: SizeInt;
  Bigger: array of T;
begin
  if FCount = Length(FItems) then
  begin
    SetLength(Bigger, FCount * 2 + 4);
    for I := 0 to FCount - 1 do
      Bigger[I] := FItems[(FHead + I) mod Length(FItems)];
    FItems := Bigger;
    FHead := 0;
  end;
  FItems[(FHead + FCount) mod Length(FItems)] := Value;
  Inc(FCount);
end;

function TQueue.Dequeue: T;
begin
  if FCount = 0 then
    raise EListError.Create('Queue is empty');
  Result := FItems[FHead];
  FItems[FHead] := Default(T);
  FHead := (FHead + 1) mod Length(FItems);
  Dec(FCount);
end;

function TQueue.Peek: T;
begin
  if FCount = 0 then
    raise EListError.Create('Queue is empty');
  Result := FItems[FHead];
end;

function TQueue.Extract: T;
begin
  Result := Dequeue;
end;

procedure TQueue.Clear;
begin
  SetLength(FItems, 0);
  FHead := 0;
  FCount := 0;
end;

function TQueue.ToArray: specialize TArray<T>;
var
  I: SizeInt;
begin
  SetLength(Result, FCount);
  for I := 0 to FCount - 1 do
    Result[I] := FItems[(FHead + I) mod Length(FItems)];
end;

{ TCollectionSnapshot }

function TCollectionSnapshot.GetCount: SizeInt;
begin
  Result := Length(FItems);
end;

function TCollectionSnapshot.ToArray: specialize TArray<T>;
begin
  Result := Copy(FItems);
end;

function TCollectionSnapshot.GetEnumerator: specialize TListEnumerator<T>;
begin
  Result := specialize TListEnumerator<T>.Create(FItems, Length(FItems));
end;

{ TDictionaryEnumerator }

constructor TDictionaryEnumerator.Create(const AKeys: array of TKey; const AValues: array of TValue);
var
  I: SizeInt;
begin
  inherited Create;
  SetLength(FPairs, Length(AKeys));
  for I := 0 to High(AKeys) do
  begin
    FPairs[I].Key := AKeys[I];
    FPairs[I].Value := AValues[I];
  end;
  FPosition := -1;
end;

function TDictionaryEnumerator.GetCurrent: specialize TPair<TKey, TValue>;
begin
  Result := FPairs[FPosition];
end;

function TDictionaryEnumerator.MoveNext: Boolean;
begin
  Inc(FPosition);
  Result := FPosition < Length(FPairs);
end;

{ TDictionary }

function TDictionary.Slot(const Key: TKey): SizeInt;
var
  Mask: SizeInt;
begin
  if Length(FKeys) = 0 then
    Exit(-1);
  Mask := Length(FKeys) - 1;
  Result := SizeInt(HashOf(Key) and QWord(Mask));
  if FComparer = nil then
    while FUsed[Result] and not BrutoEquals(FKeys[Result], Key) do
      Result := (Result + 1) and Mask
  else
    while FUsed[Result] and not FComparer.Equals(FKeys[Result], Key) do
      Result := (Result + 1) and Mask;
end;

function TDictionary.HashOf(const Key: TKey): QWord;
begin
  if FComparer = nil then
    Result := BrutoHash(Key)
  else
    Result := FComparer.GetHashCode(Key);
end;

constructor TDictionary.Create;
begin
  inherited Create;
end;

constructor TDictionary.Create(ACapacity: SizeInt);
begin
  inherited Create;
end;

constructor TDictionary.Create(const AComparer: specialize IEqualityComparer<TKey>);
begin
  inherited Create;
  FComparer := AComparer;
end;

constructor TDictionary.Create(ACapacity: SizeInt;
  const AComparer: specialize IEqualityComparer<TKey>);
begin
  inherited Create;
  FComparer := AComparer;
end;

procedure TDictionary.Grow;
var
  OldKeys: array of TKey;
  OldValues: array of TValue;
  OldUsed: array of Boolean;
  I, S: SizeInt;
begin
  OldKeys := FKeys;
  OldValues := FValues;
  OldUsed := FUsed;
  FKeys := nil;
  FValues := nil;
  FUsed := nil;
  if Length(OldKeys) = 0 then
    S := 16
  else
    S := Length(OldKeys) * 2;
  SetLength(FKeys, S);
  SetLength(FValues, S);
  SetLength(FUsed, S);
  for I := 0 to High(OldKeys) do
    if OldUsed[I] then
    begin
      S := Slot(OldKeys[I]);
      FKeys[S] := OldKeys[I];
      FValues[S] := OldValues[I];
      FUsed[S] := True;
    end;
end;

function TDictionary.TryAdd(const Key: TKey; const Value: TValue): Boolean;
var
  S: SizeInt;
begin
  if (FCount + 1) * 4 > Length(FKeys) * 3 then
    Grow;
  S := Slot(Key);
  Result := not FUsed[S];
  if Result then
  begin
    FKeys[S] := Key;
    FValues[S] := Value;
    FUsed[S] := True;
    Inc(FCount);
  end;
end;

procedure TDictionary.Add(const Key: TKey; const Value: TValue);
begin
  if not TryAdd(Key, Value) then
    raise EListError.Create('Dupliate key');
end;

procedure TDictionary.AddOrSetValue(const Key: TKey; const Value: TValue);
var
  S: SizeInt;
begin
  if not TryAdd(Key, Value) then
  begin
    S := Slot(Key);
    FValues[S] := Value;
  end;
end;

procedure TDictionary.Remove(const Key: TKey);
var
  S, Next, Home, Mask: SizeInt;
begin
  S := Slot(Key);
  if (S < 0) or not FUsed[S] then
    Exit;
  Mask := Length(FKeys) - 1;
  FUsed[S] := False;
  FKeys[S] := Default(TKey);
  FValues[S] := Default(TValue);
  Dec(FCount);
  { Move back the entries of the run after it that hash before it. }
  Next := (S + 1) and Mask;
  while FUsed[Next] do
  begin
    Home := SizeInt(HashOf(FKeys[Next]) and QWord(Mask));
    if ((Next - Home) and Mask) >= ((Next - S) and Mask) then
    begin
      FKeys[S] := FKeys[Next];
      FValues[S] := FValues[Next];
      FUsed[S] := True;
      FUsed[Next] := False;
      FKeys[Next] := Default(TKey);
      FValues[Next] := Default(TValue);
      S := Next;
    end;
    Next := (Next + 1) and Mask;
  end;
end;

procedure TDictionary.Clear;
begin
  FKeys := nil;
  FValues := nil;
  FUsed := nil;
  FCount := 0;
end;

function TDictionary.TryGetValue(const Key: TKey; out Value: TValue): Boolean;
var
  S: SizeInt;
begin
  S := Slot(Key);
  Result := (S >= 0) and FUsed[S];
  if Result then
    Value := FValues[S]
  else
    Value := Default(TValue);
end;

function TDictionary.ContainsKey(const Key: TKey): Boolean;
var
  S: SizeInt;
begin
  S := Slot(Key);
  Result := (S >= 0) and FUsed[S];
end;

function TDictionary.ContainsValue(const Value: TValue): Boolean;
var
  I: SizeInt;
begin
  Result := False;
  for I := 0 to High(FKeys) do
    if FUsed[I] and BrutoEquals(FValues[I], Value) then
      Exit(True);
end;

function TDictionary.GetItem(const Key: TKey): TValue;
begin
  if not TryGetValue(Key, Result) then
    raise EListError.Create('Key not found');
end;

procedure TDictionary.SetItem(const Key: TKey; const Value: TValue);
var
  S: SizeInt;
begin
  S := Slot(Key);
  if (S < 0) or not FUsed[S] then
    raise EListError.Create('Key not found');
  FValues[S] := Value;
end;

destructor TDictionary.Destroy;
begin
  FKeyCollection.Free;
  FValueCollection.Free;
  inherited Destroy;
end;

function TDictionary.GetKeys: specialize TCollectionSnapshot<TKey>;
begin
  if FKeyCollection = nil then
    FKeyCollection := specialize TCollectionSnapshot<TKey>.Create;
  FKeyCollection.FItems := KeyArray;
  Result := FKeyCollection;
end;

function TDictionary.GetValues: specialize TCollectionSnapshot<TValue>;
begin
  if FValueCollection = nil then
    FValueCollection := specialize TCollectionSnapshot<TValue>.Create;
  FValueCollection.FItems := ValueArray;
  Result := FValueCollection;
end;

function TDictionary.KeyArray: specialize TArray<TKey>;
var
  I, N: SizeInt;
begin
  SetLength(Result, FCount);
  N := 0;
  for I := 0 to High(FKeys) do
    if FUsed[I] then
    begin
      Result[N] := FKeys[I];
      Inc(N);
    end;
end;

function TDictionary.ValueArray: specialize TArray<TValue>;
var
  I, N: SizeInt;
begin
  SetLength(Result, FCount);
  N := 0;
  for I := 0 to High(FKeys) do
    if FUsed[I] then
    begin
      Result[N] := FValues[I];
      Inc(N);
    end;
end;

function TDictionary.ToArray: specialize TArray<specialize TPair<TKey, TValue>>;
var
  I, N: SizeInt;
begin
  SetLength(Result, FCount);
  N := 0;
  for I := 0 to High(FKeys) do
    if FUsed[I] then
    begin
      Result[N].Key := FKeys[I];
      Result[N].Value := FValues[I];
      Inc(N);
    end;
end;

function TDictionary.GetEnumerator: specialize TDictionaryEnumerator<TKey, TValue>;
begin
  Result := specialize TDictionaryEnumerator<TKey, TValue>.Create(KeyArray, ValueArray);
end;

end.
